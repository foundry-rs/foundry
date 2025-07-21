use std::ops::{Deref, DerefMut};

use crate::{
    backend::DatabaseExt, constants::DEFAULT_CREATE2_DEPLOYER_CODEHASH, Env, InspectorExt,
};
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_evm::{
    eth::EthEvmContext,
    precompiles::{DynPrecompile, PrecompilesMap},
    rwasm_revm::{RwasmEvm},
    Evm, EvmEnv,
};
use alloy_evm::rwasm_revm::RwasmFrame;
use alloy_primitives::{Address, Bytes, U256};
use foundry_fork_db::DatabaseError;
use revm::{
    context::{
        result::{EVMError, HaltReason, ResultAndState},
        BlockEnv, CfgEnv, ContextTr, CreateScheme, Evm as RevmRwasm, JournalTr, LocalContext,
        TxEnv,
    },
    handler::{
        instructions::EthInstructions, EthPrecompiles, FrameInitOrResult, FrameResult, Handler,
        ItemOrResult, MainnetHandler,
    },
    inspector::InspectorHandler,
    interpreter::{
        interpreter::EthInterpreter, return_ok, CallInput, CallInputs, CallOutcome, CallScheme,
        CallValue, CreateInputs, CreateOutcome, EOFCreateKind, FrameInput, Gas, InstructionResult,
        InterpreterResult,
    },
    precompile::{secp256r1::P256VERIFY, PrecompileSpecId, Precompiles},
    primitives::hardfork::SpecId,
    state::Bytecode,
    Context, ExecuteEvm, Journal,
};

pub fn new_evm_with_inspector<'i, 'db, I: InspectorExt + ?Sized>(
    db: &'db mut dyn DatabaseExt,
    env: Env,
    inspector: &'i mut I,
) -> FoundryEvm<'db, &'i mut I> {
    let ctx = EthEvmContext {
        journaled_state: {
            let mut journal = Journal::new(db);
            journal.set_spec_id(env.evm_env.cfg_env.spec);
            journal
        },
        block: env.evm_env.block_env,
        cfg: env.evm_env.cfg_env,
        tx: env.tx,
        chain: (),
        local: LocalContext::default(),
        error: Ok(()),
    };
    let spec = ctx.cfg.spec;

    let mut evm =
        FoundryEvm { inner: RwasmEvm::new(ctx, inspector).with_precompiles(get_precompiles(spec)) };

    inject_precompiles(&mut evm);

    evm
}

pub fn new_evm_with_existing_context<'a>(
    ctx: EthEvmContext<&'a mut dyn DatabaseExt>,
    inspector: &'a mut dyn InspectorExt,
) -> FoundryEvm<'a, &'a mut dyn InspectorExt> {
    let spec = ctx.cfg.spec;

    let mut evm =
        FoundryEvm { inner: RwasmEvm::new(ctx, inspector).with_precompiles(get_precompiles(spec)) };

    inject_precompiles(&mut evm);

    evm
}

/// Conditionally inject additional precompiles into the EVM context.
fn inject_precompiles(evm: &mut FoundryEvm<'_, impl InspectorExt>) {
    if evm.inspector().is_odyssey() {
        evm.precompiles_mut().apply_precompile(P256VERIFY.address(), |_| {
            Some(DynPrecompile::from(P256VERIFY.precompile()))
        });
    }
}

/// Get the precompiles for the given spec.
fn get_precompiles(spec: SpecId) -> PrecompilesMap {
    PrecompilesMap::from_static(
        EthPrecompiles {
            precompiles: Precompiles::new(PrecompileSpecId::from_spec_id(spec)),
            spec,
        }
        .precompiles,
    )
}

/// Get the call inputs for the CREATE2 factory.
fn get_create2_factory_call_inputs(
    salt: U256,
    inputs: &CreateInputs,
    deployer: Address,
) -> CallInputs {
    let calldata = [&salt.to_be_bytes::<32>()[..], &inputs.init_code[..]].concat();
    CallInputs {
        caller: inputs.caller,
        bytecode_address: deployer,
        target_address: deployer,
        scheme: CallScheme::Call,
        value: CallValue::Transfer(inputs.value),
        input: CallInput::Bytes(calldata.into()),
        gas_limit: inputs.gas_limit,
        is_static: false,
        return_memory_offset: 0..0,
        is_eof: false,
    }
}

pub struct FoundryEvm<'db, I: InspectorExt> {
    #[allow(clippy::type_complexity)]
    pub inner: RwasmEvm<
        EthEvmContext<&'db mut dyn DatabaseExt>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
        PrecompilesMap,
    >,
}

impl<I: InspectorExt> FoundryEvm<'_, I> {
    pub fn run_execution(
        &mut self,
        frame: FrameInput,
    ) -> Result<FrameResult, EVMError<DatabaseError>> {
        println!("DEBUG evm/core/evm run_execution: frame {:?}", frame);

        let bytecode: Option<Bytes> = match &frame {
            FrameInput::Create(create) => Some(create.init_code.clone()),
            FrameInput::EOFCreate(eof_create) => match &eof_create.kind {
                EOFCreateKind::Tx { initdata } => Some(initdata.clone()),
                _ => None,
            },
            _ => None,
        };
        println!("bytecode: {:?}", bytecode);
        // TODO(d1r1): We need to get rwasm execution result here somehow. In previous version we
        // were able to do it as is. If i remember correct. So in the new version we need to

        let is_rwasm =
            bytecode.as_ref().map_or(false, |bytes| bytes.starts_with(&[0x00, 0x61, 0x73, 0x6D]));

        if is_rwasm {
            println!("DEBUG: Detected RWASM contract, switching execution logic");

            // let action = run_rwasm_loop(&mut rwasm_frame, &mut self.inner)?;

            // return match action {
            //     InterpreterAction::Return { result } => Ok(FrameResult {
            //         gas: result.gas,
            //         output: result.output,
            //         result: result.result,
            //     }),
            //     _ => Err(EVMError::Custom("Unexpected action".into())),
            // };
        }
        let mut handler = FoundryHandler::<_>::default();
        let frame = handler.inspect_first_frame_init(&mut self.inner.0, frame)?;
        let frame_result = match frame {
            ItemOrResult::Item(frame) => handler.inspect_run_exec_loop(&mut self.inner.0, frame)?,
            ItemOrResult::Result(result) => result,
        };

        Ok(frame_result)
    }
}

impl<'db, I: InspectorExt> Evm for FoundryEvm<'db, I> {
    type Precompiles = PrecompilesMap;
    type Inspector = I;
    type DB = &'db mut dyn DatabaseExt;
    type Error = EVMError<DatabaseError>;
    type HaltReason = HaltReason;
    type Spec = SpecId;
    type Tx = TxEnv;

    fn chain_id(&self) -> u64 {
        self.inner.0.ctx.cfg.chain_id
    }

    fn block(&self) -> &BlockEnv {
        &self.inner.0.block
    }

    fn db_mut(&mut self) -> &mut Self::DB {
        self.inner.0.db()
    }

    fn precompiles(&self) -> &Self::Precompiles {
        &self.inner.0.precompiles
    }

    fn precompiles_mut(&mut self) -> &mut Self::Precompiles {
        &mut self.inner.0.precompiles
    }

    fn inspector(&self) -> &Self::Inspector {
        &self.inner.0.inspector
    }

    fn inspector_mut(&mut self) -> &mut Self::Inspector {
        &mut self.inner.0.inspector
    }

    fn set_inspector_enabled(&mut self, _enabled: bool) {
        unimplemented!("FoundryEvm is always inspecting")
    }

    fn transact_raw(
        &mut self,
        tx: Self::Tx,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        let mut handler = FoundryHandler::<_>::default();
        self.inner.set_tx(tx);
        handler.inspect_run(&mut self.inner.0)
    }

    fn transact_system_call(
        &mut self,
        _caller: Address,
        _contract: Address,
        _data: Bytes,
    ) -> Result<ResultAndState<Self::HaltReason>, Self::Error> {
        unimplemented!()
    }

    fn finish(self) -> (Self::DB, EvmEnv<Self::Spec>)
    where
        Self: Sized,
    {
        let Context { block: block_env, cfg: cfg_env, journaled_state, .. } = self.inner.0.ctx;

        (journaled_state.database, EvmEnv { block_env, cfg_env })
    }
}

impl<'db, I: InspectorExt> Deref for FoundryEvm<'db, I> {
    type Target = Context<BlockEnv, TxEnv, CfgEnv, &'db mut dyn DatabaseExt>;

    fn deref(&self) -> &Self::Target {
        &self.inner.0.ctx
    }
}

impl<I: InspectorExt> DerefMut for FoundryEvm<'_, I> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.0.ctx
    }
}

pub struct FoundryHandler<'db, I: InspectorExt> {
    #[allow(clippy::type_complexity)]
    inner: MainnetHandler<
        RevmRwasm<
            EthEvmContext<&'db mut dyn DatabaseExt>,
            I,
            EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
            PrecompilesMap,
        >,
        EVMError<DatabaseError>,
        RwasmFrame<
            RevmRwasm<
                EthEvmContext<&'db mut dyn DatabaseExt>,
                I,
                EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
                PrecompilesMap,
            >,
            EVMError<DatabaseError>,
            EthInterpreter,
        >,
    >,
    create2_overrides: Vec<(usize, CallInputs)>,
}

impl<I: InspectorExt> Default for FoundryHandler<'_, I> {
    fn default() -> Self {
        Self { inner: MainnetHandler::default(), create2_overrides: Vec::new() }
    }
}

impl<'db, I: InspectorExt> Handler for FoundryHandler<'db, I> {
    type Evm = RevmRwasm<
        EthEvmContext<&'db mut dyn DatabaseExt>,
        I,
        EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
        PrecompilesMap,
    >;
    type Error = EVMError<DatabaseError>;
    type Frame = RwasmFrame<
        RevmRwasm<
            EthEvmContext<&'db mut dyn DatabaseExt>,
            I,
            EthInstructions<EthInterpreter, EthEvmContext<&'db mut dyn DatabaseExt>>,
            PrecompilesMap,
        >,
        EVMError<DatabaseError>,
        EthInterpreter,
    >;
    type HaltReason = HaltReason;

    fn frame_return_result(
        &mut self,
        frame: &mut Self::Frame,
        evm: &mut Self::Evm,
        result: <Self::Frame as revm::handler::Frame>::FrameResult,
    ) -> Result<(), Self::Error> {
        println!("DEBUG: crates/evm/core/evm.rs frame_return_result");
        let result = if self
            .create2_overrides
            .last()
            .is_some_and(|(depth, _)| *depth == evm.journal().depth)
        {
            println!("DEBUG2: crates/evm/core/evm.rs frame_return_result");

            let (_, call_inputs) = self.create2_overrides.pop().unwrap();
            let FrameResult::Call(mut result) = result else {
                unreachable!("create2 override should be a call frame");
            };

            // Decode address from output.
            let address = match result.instruction_result() {
                return_ok!() => Address::try_from(result.output().as_ref())
                    .map_err(|_| {
                        result.result = InterpreterResult {
                            result: InstructionResult::Revert,
                            output: "invalid CREATE2 factory output".into(),
                            gas: Gas::new(call_inputs.gas_limit),
                        };
                    })
                    .ok(),
                _ => None,
            };

            FrameResult::Create(CreateOutcome { result: result.result, address })
        } else {
            result
        };

        println!("DEBUG3: crates/evm/core/evm.rs inner.frame_return_result");

        self.inner.frame_return_result(frame, evm, result)
    }
}

impl<I: InspectorExt> InspectorHandler for FoundryHandler<'_, I> {
    type IT = EthInterpreter;

    fn inspect_frame_call(
        &mut self,
        frame: &mut Self::Frame,
        evm: &mut Self::Evm,
    ) -> Result<FrameInitOrResult<Self::Frame>, Self::Error> {
        let frame_or_result = self.inner.inspect_frame_call(frame, evm)?;

        let ItemOrResult::Item(FrameInput::Create(inputs)) = &frame_or_result else {
            return Ok(frame_or_result);
        };

        let CreateScheme::Create2 { salt } = inputs.scheme else { return Ok(frame_or_result) };

        if !evm.inspector.should_use_create2_factory(&mut evm.ctx, inputs) {
            return Ok(frame_or_result);
        }

        let gas_limit = inputs.gas_limit;

        // Get CREATE2 deployer.
        let create2_deployer = evm.inspector.create2_deployer();

        // Generate call inputs for CREATE2 factory.
        let call_inputs = get_create2_factory_call_inputs(salt, &inputs, create2_deployer);

        // Push data about current override to the stack.
        self.create2_overrides.push((evm.journal().depth(), call_inputs.clone()));

        // Sanity check that CREATE2 deployer exists.
        let code_hash = evm.journal().load_account(create2_deployer)?.info.code_hash;
        if code_hash == KECCAK_EMPTY {
            return Ok(ItemOrResult::Result(FrameResult::Call(CallOutcome {
                result: InterpreterResult {
                    result: InstructionResult::Revert,
                    output: Bytes::copy_from_slice(
                        format!("missing CREATE2 deployer: {create2_deployer}").as_bytes(),
                    ),
                    gas: Gas::new(gas_limit),
                },
                memory_offset: 0..0,
            })));
        } else if code_hash != DEFAULT_CREATE2_DEPLOYER_CODEHASH {
            return Ok(ItemOrResult::Result(FrameResult::Call(CallOutcome {
                result: InterpreterResult {
                    result: InstructionResult::Revert,
                    output: "invalid CREATE2 deployer bytecode".into(),
                    gas: Gas::new(gas_limit),
                },
                memory_offset: 0..0,
            })));
        }

        // Return the created CALL frame instead
        Ok(ItemOrResult::Item(FrameInput::Call(Box::new(call_inputs))))
    }
}
