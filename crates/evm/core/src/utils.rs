pub use crate::ic::*;
use crate::{
    backend::DatabaseExt, constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH, precompiles::ODYSSEY_P256,
    Env, InspectorExt,
};
use alloy_consensus::BlockHeader;
use alloy_evm::eth::EthEvmContext;
use alloy_json_abi::{Function, JsonAbi};
use alloy_network::{AnyTxEnvelope, TransactionResponse};
use alloy_primitives::{Address, Bytes, Selector, TxKind, B256, U256};
use alloy_provider::{network::BlockResponse, Network};
use alloy_rpc_types::{Transaction, TransactionRequest};
use foundry_common::is_impersonated_tx;
use foundry_config::NamedChain;
use foundry_fork_db::DatabaseError;
use revm::{
    context::{ContextTr, Evm, EvmData, JournalInner},
    context_interface::{result::EVMError, CreateScheme},
    handler::{
        instructions::EthInstructions, EthPrecompiles, FrameOrResult, FrameResult, Handler,
        PrecompileProvider,
    },
    interpreter::{
        interpreter::EthInterpreter, return_ok, CallInputs, CallOutcome, CallScheme, CallValue,
        CreateInputs, CreateOutcome, Gas, InstructionResult, InterpreterResult,
    },
    precompile::PrecompileError,
    primitives::{hardfork::SpecId, KECCAK_EMPTY},
    Journal,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub use revm::state::EvmState as StateChangeset;

/// Depending on the configured chain id and block number this should apply any specific changes
///
/// - checks for prevrandao mixhash after merge
/// - applies chain specifics: on Arbitrum `block.number` is the L1 block
///
/// Should be called with proper chain id (retrieved from provider if not provided).
pub fn apply_chain_and_block_specific_env_changes<N: Network>(
    env: &mut crate::Env,
    block: &N::BlockResponse,
) {
    use NamedChain::*;
    if let Ok(chain) = NamedChain::try_from(env.evm_env.cfg_env.chain_id) {
        let block_number = block.header().number();

        match chain {
            Mainnet => {
                // after merge difficulty is supplanted with prevrandao EIP-4399
                if block_number >= 15_537_351u64 {
                    env.evm_env.block_env.difficulty =
                        env.evm_env.block_env.prevrandao.unwrap_or_default().into();
                }

                return;
            }
            BinanceSmartChain | BinanceSmartChainTestnet => {
                // https://github.com/foundry-rs/foundry/issues/9942
                // As far as observed from the source code of bnb-chain/bsc, the `difficulty` field
                // is still in use and returned by the corresponding opcode but `prevrandao`
                // (`mixHash`) is always zero, even though bsc adopts the newer EVM
                // specification. This will confuse revm and causes emulation
                // failure.
                env.evm_env.block_env.prevrandao = Some(env.evm_env.block_env.difficulty.into());
                return;
            }
            Moonbeam | Moonbase | Moonriver | MoonbeamDev => {
                if env.evm_env.block_env.prevrandao.is_none() {
                    // <https://github.com/foundry-rs/foundry/issues/4232>
                    env.evm_env.block_env.prevrandao = Some(B256::random());
                }
            }
            c if c.is_arbitrum() => {
                // on arbitrum `block.number` is the L1 block which is included in the
                // `l1BlockNumber` field
                if let Some(l1_block_number) = block
                    .other_fields()
                    .and_then(|other| other.get("l1BlockNumber").cloned())
                    .and_then(|v| v.as_u64())
                {
                    env.evm_env.block_env.number = l1_block_number;
                }
            }
            _ => {}
        }
    }

    // if difficulty is `0` we assume it's past merge
    if block.header().difficulty().is_zero() {
        env.evm_env.block_env.difficulty =
            env.evm_env.block_env.prevrandao.unwrap_or_default().into();
    }
}

/// Given an ABI and selector, it tries to find the respective function.
pub fn get_function<'a>(
    contract_name: &str,
    selector: Selector,
    abi: &'a JsonAbi,
) -> eyre::Result<&'a Function> {
    abi.functions()
        .find(|func| func.selector() == selector)
        .ok_or_else(|| eyre::eyre!("{contract_name} does not have the selector {selector}"))
}

/// Configures the env for the given RPC transaction.
/// Accounts for an impersonated transaction by resetting the `env.tx.caller` field to `tx.from`.
pub fn configure_tx_env(env: &mut crate::Env, tx: &Transaction<AnyTxEnvelope>) {
    let impersonated_from = is_impersonated_tx(&tx.inner).then_some(tx.from());
    if let AnyTxEnvelope::Ethereum(tx) = &tx.inner.inner() {
        configure_tx_req_env(env, &tx.clone().into(), impersonated_from).expect("cannot fail");
    }
}

/// Configures the env for the given RPC transaction request.
/// `impersonated_from` is the address of the impersonated account. This helps account for an
/// impersonated transaction by resetting the `env.tx.caller` field to `impersonated_from`.
pub fn configure_tx_req_env(
    env: &mut crate::Env,
    tx: &TransactionRequest,
    impersonated_from: Option<Address>,
) -> eyre::Result<()> {
    let TransactionRequest {
        nonce,
        from,
        to,
        value,
        gas_price,
        gas,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        max_fee_per_blob_gas,
        ref input,
        chain_id,
        ref blob_versioned_hashes,
        ref access_list,
        transaction_type: _,
        ref authorization_list,
        sidecar: _,
    } = *tx;

    // If no `to` field then set create kind: https://eips.ethereum.org/EIPS/eip-2470#deployment-transaction
    env.tx.kind = to.unwrap_or(TxKind::Create);
    // If the transaction is impersonated, we need to set the caller to the from
    // address Ref: https://github.com/foundry-rs/foundry/issues/9541
    env.tx.caller =
        impersonated_from.unwrap_or(from.ok_or_else(|| eyre::eyre!("missing `from` field"))?);
    env.tx.gas_limit = gas.ok_or_else(|| eyre::eyre!("missing `gas` field"))?;
    env.tx.nonce = nonce.unwrap_or_default();
    env.tx.value = value.unwrap_or_default();
    env.tx.data = input.input().cloned().unwrap_or_default();
    env.tx.chain_id = chain_id;

    // Type 1, EIP-2930
    env.tx.access_list = access_list.clone().unwrap_or_default();

    // Type 2, EIP-1559
    env.tx.gas_price = gas_price.or(max_fee_per_gas).unwrap_or_default();
    env.tx.gas_priority_fee = max_priority_fee_per_gas;

    // Type 3, EIP-4844
    env.tx.blob_hashes = blob_versioned_hashes.clone().unwrap_or_default();
    env.tx.max_fee_per_blob_gas = max_fee_per_blob_gas.unwrap_or_default();

    // Type 4, EIP-7702
    if let Some(authorization_list) = authorization_list {
        env.tx.authorization_list = authorization_list.clone();
    }

    Ok(())
}

/// Get the gas used, accounting for refunds
pub fn gas_used(spec: SpecId, spent: u64, refunded: u64) -> u64 {
    let refund_quotient = if SpecId::is_enabled_in(spec, SpecId::LONDON) { 5 } else { 2 };
    spent - (refunded).min(spent / refund_quotient)
}

fn get_create2_factory_call_inputs(
    salt: U256,
    inputs: CreateInputs,
    deployer: Address,
) -> CallInputs {
    let calldata = [&salt.to_be_bytes::<32>()[..], &inputs.init_code[..]].concat();
    CallInputs {
        caller: inputs.caller,
        bytecode_address: deployer,
        target_address: deployer,
        scheme: CallScheme::Call,
        value: CallValue::Transfer(inputs.value),
        input: calldata.into(),
        gas_limit: inputs.gas_limit,
        is_static: false,
        return_memory_offset: 0..0,
        is_eof: false,
    }
}

/// Used for routing certain CREATE2 invocations through CREATE2_DEPLOYER.
///
/// Overrides create hook with CALL frame if [InspectorExt::should_use_create2_factory] returns
/// true. Keeps track of overridden frames and handles outcome in the overridden
/// insert_call_outcome hook by inserting decoded address directly into interpreter.
///
/// Should be installed after [revm::inspector_handle_register] and before any other registers.
// pub fn create2_handler_register<I: InspectorExt>(
//     handler: &mut Handler<'_, I, &mut dyn DatabaseExt>,
// ) {
//     let create2_overrides = Rc::<RefCell<Vec<_>>>::new(RefCell::new(Vec::new()));

//     let create2_overrides_inner = create2_overrides.clone();
//     let old_handle = handler.execution.create.clone();
//     handler.execution.create =
//         Arc::new(move |ctx, mut inputs| -> Result<FrameOrResult, EVMError<DatabaseError>> {
//             let CreateScheme::Create2 { salt } = inputs.scheme else {
//                 return old_handle(ctx, inputs);
//             };
//             if !ctx.external.should_use_create2_factory(&mut ctx.evm, &mut inputs) {
//                 return old_handle(ctx, inputs);
//             }

//             let gas_limit = inputs.gas_limit;

//             // Get CREATE2 deployer.
//             let create2_deployer = ctx.external.create2_deployer();
//             // Generate call inputs for CREATE2 factory.
//             let mut call_inputs = get_create2_factory_call_inputs(salt, *inputs,
// create2_deployer);

//             // Call inspector to change input or return outcome.
//             let outcome = ctx.external.call(&mut ctx.evm, &mut call_inputs);

//             // Push data about current override to the stack.
//             create2_overrides_inner
//                 .borrow_mut()
//                 .push((ctx.evm.journaled_state.depth(), call_inputs.clone()));

//             // Sanity check that CREATE2 deployer exists.
//             let code_hash = ctx.evm.load_account(create2_deployer)?.info.code_hash;
//             if code_hash == KECCAK_EMPTY {
//                 return Ok(FrameOrResult::Result(FrameResult::Call(CallOutcome {
//                     result: InterpreterResult {
//                         result: InstructionResult::Revert,
//                         output: format!("missing CREATE2 deployer: {create2_deployer}").into(),
//                         gas: Gas::new(gas_limit),
//                     },
//                     memory_offset: 0..0,
//                 })))
//             } else if code_hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
//                 return Ok(FrameOrResult::Result(FrameResult::Call(CallOutcome {
//                     result: InterpreterResult {
//                         result: InstructionResult::Revert,
//                         output: "invalid CREATE2 deployer bytecode".into(),
//                         gas: Gas::new(gas_limit),
//                     },
//                     memory_offset: 0..0,
//                 })))
//             }

//             // Handle potential inspector override.
//             if let Some(outcome) = outcome {
//                 return Ok(FrameOrResult::Result(FrameResult::Call(outcome)));
//             }

//             // Create CALL frame for CREATE2 factory invocation.
//             let mut frame_or_result = ctx.evm.make_call_frame(&call_inputs);

//             if let Ok(FrameOrResult::Item(frame)) = &mut frame_or_result {
//                 ctx.external
//                     .initialize_interp(&mut frame.frame_data_mut().interpreter, &mut ctx.evm)
//             }
//             frame_or_result
//         });

//     let create2_overrides_inner = create2_overrides;
//     let old_handle = handler.execution.insert_call_outcome.clone();
//     handler.execution.insert_call_outcome =
//         Arc::new(move |ctx, frame, shared_memory, mut outcome| {
//             // If we are on the depth of the latest override, handle the outcome.
//             if create2_overrides_inner
//                 .borrow()
//                 .last()
//                 .is_some_and(|(depth, _)| *depth == ctx.evm.journaled_state.depth())
//             {
//                 let (_, call_inputs) = create2_overrides_inner.borrow_mut().pop().unwrap();
//                 outcome = ctx.external.call_end(&mut ctx.evm, &call_inputs, outcome);

//                 // Decode address from output.
//                 let address = match outcome.instruction_result() {
//                     return_ok!() => Address::try_from(outcome.output().as_ref())
//                         .map_err(|_| {
//                             outcome.result = InterpreterResult {
//                                 result: InstructionResult::Revert,
//                                 output: "invalid CREATE2 factory output".into(),
//                                 gas: Gas::new(call_inputs.gas_limit),
//                             };
//                         })
//                         .ok(),
//                     _ => None,
//                 };
//                 frame
//                     .frame_data_mut()
//                     .interpreter
//                     .insert_create_outcome(CreateOutcome { address, result: outcome.result });

//                 Ok(())
//             } else {
//                 old_handle(ctx, frame, shared_memory, outcome)
//             }
//         });
// }

/// [`PrecompileProvider`] wrapper that enables [`P256VERIFY`] if `odyssey` is enabled.
pub struct MaybeOdysseyPrecompiles {
    inner: EthPrecompiles,
    odyssey: bool,
}

impl MaybeOdysseyPrecompiles {
    /// Creates a new instance of the [`MaybeOdysseyPrecompiles`].
    pub fn new(odyssey: bool) -> Self {
        Self { inner: EthPrecompiles::default(), odyssey }
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for MaybeOdysseyPrecompiles {
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <<CTX as ContextTr>::Cfg as revm::context::Cfg>::Spec) {
        PrecompileProvider::<CTX>::set_spec(&mut self.inner, spec);
    }

    fn run(
        &mut self,
        context: &mut CTX,
        address: &Address,
        bytes: &Bytes,
        gas_limit: u64,
    ) -> Result<Option<Self::Output>, String> {
        if self.odyssey && address == ODYSSEY_P256.address() {
            let mut result = InterpreterResult {
                result: InstructionResult::Return,
                gas: Gas::new(gas_limit),
                output: Bytes::new(),
            };

            match ODYSSEY_P256.precompile()(bytes, gas_limit) {
                Ok(output) => {
                    let underflow = result.gas.record_cost(output.gas_used);
                    if underflow {
                        result.result = InstructionResult::PrecompileOOG;
                    } else {
                        result.result = InstructionResult::Return;
                        result.output = output.bytes;
                    }
                }
                Err(e) => {
                    if let PrecompileError::Fatal(_) = e {
                        return Err(e.to_string());
                    }
                    result.result = if e.is_oog() {
                        InstructionResult::PrecompileOOG
                    } else {
                        InstructionResult::PrecompileError
                    };
                }
            }
        }

        self.inner.run(context, address, bytes, gas_limit)
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        let warm_addresses = self.inner.warm_addresses() as Box<dyn Iterator<Item = Address>>;

        let iter = if self.odyssey {
            Box::new(warm_addresses.chain(core::iter::once(*ODYSSEY_P256.address())))
        } else {
            warm_addresses
        };

        Box::new(iter)
    }

    fn contains(&self, address: &Address) -> bool {
        if self.odyssey && address == ODYSSEY_P256.address() {
            true
        } else {
            self.inner.contains(address)
        }
    }
}

/// [`revm::Context`] type used by Foundry.
pub type FoundryEvmCtx<'a> = EthEvmContext<&'a mut dyn DatabaseExt>;

/// Type alias for revm's EVM used by Foundry.
pub type FoundryEvm<'db, INSP> = Evm<
    FoundryEvmCtx<'db>,
    INSP,
    EthInstructions<EthInterpreter, FoundryEvmCtx<'db>>,
    MaybeOdysseyPrecompiles,
>;

/// Creates a new EVM with the given inspector.
pub fn new_evm_with_inspector<'i, 'db, I: InspectorExt + ?Sized>(
    db: &'db mut dyn DatabaseExt,
    env: &mut Env,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    new_evm_with_context(
        FoundryEvmCtx {
            journaled_state: Journal::new_with_inner(
                db,
                JournalInner::new(env.evm_env.cfg_env.spec),
            ),
            block: env.evm_env.block_env.clone(),
            cfg: env.evm_env.cfg_env.clone(),
            tx: env.tx.clone(),
            chain: (),
            error: Ok(()),
        },
        inspector,
    )
}

/// Creates a new EVM with the given context.
pub fn new_evm_with_context<'db, 'i, I: InspectorExt + ?Sized>(
    ctx: FoundryEvmCtx<'db>,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    // handler.append_handler_register_plain(create2_handler_register);

    let is_odyssey = inspector.is_odyssey();

    Evm {
        data: EvmData { ctx, inspector },
        instruction: EthInstructions::default(),
        precompiles: MaybeOdysseyPrecompiles::new(is_odyssey),
    }
}
