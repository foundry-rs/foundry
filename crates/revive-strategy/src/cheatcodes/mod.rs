use std::{
    any::{Any, TypeId},
    fmt::Debug,
    sync::{Arc, Mutex},
};

use alloy_primitives::{Address, Bytes, B256, U256};
use foundry_common::sh_err;
use foundry_compilers::resolc::dual_compiled_contracts::DualCompiledContracts;
use revive_env::{AccountId, Runtime, System};

use foundry_cheatcodes::{
    Broadcast, BroadcastableTransactions, CheatcodeInspectorStrategy,
    CheatcodeInspectorStrategyContext, CheatcodeInspectorStrategyRunner, CheatsConfig, CheatsCtxt,
    CommonCreateInput, Ecx, EvmCheatcodeInspectorStrategyRunner, InnerEcx, Result, Vm::pvmCall,
};

use polkadot_sdk::{
    frame_support::traits::{fungible::Mutate, Currency},
    pallet_balances,
    pallet_revive::{
        evm::GasEncoder, AddressMapper, BalanceOf, BalanceWithDust, BumpNonce, Code, Config,
        DepositLimit, Pallet,
    },
    polkadot_sdk_frame::prelude::OriginFor,
    sp_core::{self, H160},
    sp_io,
    sp_weights::Weight,
};

use revm::{
    interpreter::{
        opcode as op, CallInputs, CallOutcome, CreateOutcome, Gas, InstructionResult, Interpreter,
        InterpreterResult,
    },
    primitives::{CreateScheme, SignedAuthorization},
};
pub trait PvmCheatcodeInspectorStrategyBuilder {
    fn new_pvm(
        test_externalities: Arc<Mutex<sp_io::TestExternalities>>,
        dual_compiled_contracts: DualCompiledContracts,
        resolc_startup: bool,
    ) -> Self;
}
impl PvmCheatcodeInspectorStrategyBuilder for CheatcodeInspectorStrategy {
    // Creates a new PVM strategy
    fn new_pvm(
        test_externalities: Arc<Mutex<sp_io::TestExternalities>>,
        dual_compiled_contracts: DualCompiledContracts,
        resolc_startup: bool,
    ) -> Self {
        Self {
            runner: &PvmCheatcodeInspectorStrategyRunner,
            context: Box::new(PvmCheatcodeInspectorStrategyContext::new(
                test_externalities,
                dual_compiled_contracts,
                resolc_startup,
            )),
        }
    }
}

/// PVM-specific strategy context.
#[derive(Debug, Default, Clone)]
pub struct PvmCheatcodeInspectorStrategyContext {
    /// Whether we're using PVM mode
    /// Currently unused but kept for future PVM-specific logic
    pub using_pvm: bool,
    /// Whether to start in PVM mode (from config)
    pub resolc_startup: bool,
    pub revive_test_externalities: Arc<Mutex<sp_io::TestExternalities>>,
    pub dual_compiled_contracts: DualCompiledContracts,
}

impl PvmCheatcodeInspectorStrategyContext {
    pub fn new(
        revive_test_externalities: Arc<Mutex<sp_io::TestExternalities>>,
        dual_compiled_contracts: DualCompiledContracts,
        resolc_startup: bool,
    ) -> Self {
        Self {
            using_pvm: false, // Start in EVM mode by default
            resolc_startup,
            revive_test_externalities,
            dual_compiled_contracts,
        }
    }
}

impl CheatcodeInspectorStrategyContext for PvmCheatcodeInspectorStrategyContext {
    fn new_cloned(&self) -> Box<dyn CheatcodeInspectorStrategyContext> {
        Box::new(self.clone())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }
}

/// Implements [CheatcodeInspectorStrategyRunner] for PVM.
#[derive(Debug, Default, Clone)]
pub struct PvmCheatcodeInspectorStrategyRunner;

impl CheatcodeInspectorStrategyRunner for PvmCheatcodeInspectorStrategyRunner {
    fn apply_full(
        &self,
        cheatcode: &dyn foundry_cheatcodes::DynCheatcode,
        ccx: &mut CheatsCtxt<'_, '_, '_, '_>,
        executor: &mut dyn foundry_cheatcodes::CheatcodesExecutor,
    ) -> Result {
        fn is<T: std::any::Any>(t: TypeId) -> bool {
            TypeId::of::<T>() == t
        }

        match cheatcode.as_any().type_id() {
            t if is::<pvmCall>(t) => {
                let pvmCall { enabled } = cheatcode.as_any().downcast_ref().unwrap();
                if *enabled {
                    let ctx = get_context_ref_mut(ccx.state.strategy.context.as_mut());
                    select_pvm(ctx, ccx.ecx);
                } else {
                    todo!("Switch back to EVM");
                }

                Ok(Default::default())
            }
            // Not custom, just invoke the default behavior
            _ => cheatcode.dyn_apply(ccx, executor),
        }
    }

    fn base_contract_deployed(&self, _ctx: &mut dyn CheatcodeInspectorStrategyContext) {
        // PVM mode is enabled, but no special handling needed for now
        // Only intercept PVM-specific calls when needed in future implementations
    }

    fn record_broadcastable_create_transactions(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        config: Arc<CheatsConfig>,
        input: &dyn CommonCreateInput,
        ecx_inner: InnerEcx<'_, '_, '_>,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
    ) {
        // Use EVM implementation for now
        // Only intercept PVM-specific calls when needed in future implementations
        EvmCheatcodeInspectorStrategyRunner.record_broadcastable_create_transactions(
            _ctx,
            config,
            input,
            ecx_inner,
            broadcast,
            broadcastable_transactions,
        );
    }

    fn record_broadcastable_call_transactions(
        &self,
        _ctx: &mut dyn CheatcodeInspectorStrategyContext,
        config: Arc<CheatsConfig>,
        call: &CallInputs,
        ecx_inner: InnerEcx<'_, '_, '_>,
        broadcast: &Broadcast,
        broadcastable_transactions: &mut BroadcastableTransactions,
        active_delegation: &mut Option<SignedAuthorization>,
    ) {
        // Use EVM implementation for now
        // Only intercept PVM-specific calls when needed in future implementations
        EvmCheatcodeInspectorStrategyRunner.record_broadcastable_call_transactions(
            _ctx,
            config,
            call,
            ecx_inner,
            broadcast,
            broadcastable_transactions,
            active_delegation,
        );
    }

    fn post_initialize_interp(
        &self,
        ctx: &mut dyn CheatcodeInspectorStrategyContext,
        _interpreter: &mut Interpreter,
        ecx: Ecx<'_, '_, '_>,
    ) {
        let ctx = get_context_ref_mut(ctx);

        if ctx.resolc_startup && !ctx.using_pvm {
            tracing::info!("startup PVM migration initiated");
            select_pvm(ctx, ecx);
            tracing::info!("startup PVM migration completed");
        }
    }

    fn pre_step_end(
        &self,
        ctx: &mut dyn CheatcodeInspectorStrategyContext,
        interpreter: &mut Interpreter,
        _ecx: Ecx<'_, '_, '_>,
    ) -> bool {
        let ctx = get_context_ref_mut(ctx);

        if !ctx.using_pvm {
            return false;
        }

        let address = match interpreter.current_opcode() {
            op::SELFBALANCE => interpreter.contract().target_address,
            op::BALANCE => {
                if interpreter.stack.is_empty() {
                    interpreter.instruction_result = InstructionResult::StackUnderflow;
                    return true;
                }

                Address::from_word(B256::from(unsafe { interpreter.stack.pop_unsafe() }))
            }
            _ => return true,
        };

        let balance =
            ctx.revive_test_externalities.lock().unwrap().execute_with(|| {
                Pallet::<Runtime>::evm_balance(&H160::from_slice(address.as_slice()))
            });
        let balance = U256::from_limbs(balance.0);

        // Skip the current BALANCE instruction since we've already handled it
        match interpreter.stack.push(balance) {
            Ok(_) => unsafe {
                interpreter.instruction_pointer = interpreter.instruction_pointer.add(1);
            },
            Err(e) => {
                interpreter.instruction_result = e;
            }
        };

        false // Let EVM handle all operations
    }
}

fn select_pvm(ctx: &mut PvmCheatcodeInspectorStrategyContext, data: InnerEcx<'_, '_, '_>) {
    if ctx.using_pvm {
        tracing::info!("already in PVM");
        return;
    }

    tracing::info!("switching to PVM");
    ctx.using_pvm = true;
    let persistent_accounts = data.db.persistent_accounts().clone();

    for address in persistent_accounts {
        let acc = data.load_account(address).expect("just loaded above");
        let amount = acc.data.info.balance;
        let nonce = acc.data.info.nonce;

        let amount_pvm =
            sp_core::U256::from_little_endian(&amount.as_le_bytes()).min(u128::MAX.into());
        let balance_native =
            BalanceWithDust::<BalanceOf<Runtime>>::from_value::<Runtime>(amount_pvm).unwrap();
        let balance = Pallet::<Runtime>::convert_native_to_evm(balance_native);
        let amount_evm = U256::from_limbs(balance.0);
        if amount != amount_evm {
            let _ = sh_err!("Amount mismatch {amount} != {amount_evm}, Polkadot balances are u128. Test results may be incorrect.");
        }
        let min_balance = pallet_balances::Pallet::<Runtime>::minimum_balance();
        ctx.revive_test_externalities.lock().unwrap().execute_with(|| {
            let account_id =
                AccountId::to_fallback_account_id(&H160::from_slice(address.as_slice()));
            let current_nonce = System::account_nonce(&account_id);

            assert!(
                current_nonce as u64 <= nonce,
                "Cannot set nonce lower than current nonce: {current_nonce} > {nonce}"
            );

            while (System::account_nonce(&account_id) as u64) < nonce {
                System::inc_account_nonce(&account_id);
            }

            // TODO: set `dust` after we have access to `AccountInfo`.
            <Runtime as Config>::Currency::set_balance(
                &account_id,
                balance_native.into_rounded_balance().saturating_add(min_balance),
            );
        })
    }
}

impl foundry_cheatcodes::CheatcodeInspectorStrategyExt for PvmCheatcodeInspectorStrategyRunner {
    /// Try handling the `CREATE` within PVM.
    ///
    /// If `Some` is returned then the result must be returned immediately, else the call must be
    /// handled in EVM.
    fn revive_try_create(
        &self,
        state: &mut foundry_cheatcodes::Cheatcodes,
        ecx: InnerEcx<'_, '_, '_>,
        input: &dyn CommonCreateInput,
        _executor: &mut dyn foundry_cheatcodes::CheatcodesExecutor,
    ) -> Option<CreateOutcome> {
        let ctx = get_context_ref_mut(state.strategy.context.as_mut());

        if !ctx.using_pvm {
            return None;
        }

        if let Some(CreateScheme::Create) = input.scheme() {
            let caller = input.caller();
            let nonce =
                ecx.load_account(input.caller()).expect("to load caller account").info.nonce;
            let address = caller.create(nonce);
            if ecx.db.get_test_contract_address().map(|addr| address == addr).unwrap_or_default() {
                tracing::info!(
                    "running create in EVM, instead of PVM (Test Contract) {:#?}",
                    address
                );
                return None;
            }
        }

        let init_code = input.init_code();
        tracing::info!("running create in PVM");

        let find_contract = ctx
            .dual_compiled_contracts
            .find_bytecode(&init_code.0)
            .unwrap_or_else(|| panic!("failed finding contract for {init_code:?}"));

        let constructor_args = find_contract.constructor_args();
        let contract = find_contract.contract();

        let max_gas =
            <<Runtime as Config>::EthGasEncoder as GasEncoder<BalanceOf<Runtime>>>::encode(
                Default::default(),
                Weight::MAX,
                1u128 << 99,
            );
        let gas_limit = sp_core::U256::from(input.gas_limit()).min(max_gas);

        let res = ctx.revive_test_externalities.lock().unwrap().execute_with(|| {
            let origin = OriginFor::<Runtime>::signed(AccountId::to_fallback_account_id(
                &H160::from_slice(input.caller().as_slice()),
            ));
            let evm_value = sp_core::U256::from_little_endian(&input.value().as_le_bytes());

            let (gas_limit, storage_deposit_limit) =
                <<Runtime as Config>::EthGasEncoder as GasEncoder<BalanceOf<Runtime>>>::decode(
                    gas_limit,
                )
                .expect("gas limit is valid");
            let storage_deposit_limit = DepositLimit::Balance(storage_deposit_limit);
            let code = Code::Upload(contract.resolc_bytecode.as_bytes().unwrap().to_vec());
            let data = constructor_args.to_vec();
            let salt = match input.scheme() {
                Some(CreateScheme::Create2 { salt }) => Some(
                    salt.as_limbs()
                        .iter()
                        .flat_map(|&x| x.to_le_bytes())
                        .collect::<Vec<u8>>()
                        .try_into()
                        .unwrap(),
                ),
                _ => None,
            };
            let bump_nonce = BumpNonce::No;

            Pallet::<Runtime>::bare_instantiate(
                origin,
                evm_value,
                gas_limit,
                storage_deposit_limit,
                code,
                data,
                salt,
                bump_nonce,
            )
        });

        let mut gas = Gas::new(input.gas_limit());
        let gas_used =
            <<Runtime as Config>::EthGasEncoder as GasEncoder<BalanceOf<Runtime>>>::encode(
                gas_limit,
                res.gas_required,
                res.storage_deposit.charge_or_zero(),
            );
        match res.result {
            Ok(result) => {
                let _ = gas.record_cost(gas_used.as_u64());

                let outcome = if result.result.did_revert() {
                    CreateOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: result.result.data.into(),
                            gas,
                        },
                        address: None,
                    }
                } else {
                    CreateOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Return,
                            output: contract.resolc_bytecode.as_bytes().unwrap().clone(),
                            gas,
                        },
                        address: Some(Address::from_slice(result.addr.as_bytes())),
                    }
                };

                Some(outcome)
            }
            Err(e) => {
                tracing::error!("Contract creation failed: {e:#?}");
                Some(CreateOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Revert,
                        output: Bytes::from_iter(
                            format!("Contract creation failed: {e:#?}").as_bytes(),
                        ),
                        gas,
                    },
                    address: None,
                })
            }
        }
    }

    /// Try handling the `CALL` within PVM.
    ///
    /// If `Some` is returned then the result must be returned immediately, else the call must be
    /// handled in EVM.
    fn revive_try_call(
        &self,
        state: &mut foundry_cheatcodes::Cheatcodes,
        ecx: InnerEcx<'_, '_, '_>,
        call: &CallInputs,
        _executor: &mut dyn foundry_cheatcodes::CheatcodesExecutor,
    ) -> Option<CallOutcome> {
        let ctx = get_context_ref_mut(state.strategy.context.as_mut());

        if !ctx.using_pvm {
            return None;
        }

        if ecx
            .db
            .get_test_contract_address()
            .map(|addr| call.bytecode_address == addr)
            .unwrap_or_default()
        {
            tracing::info!(
                "running call in EVM, instead of PVM (Test Contract) {:#?}",
                call.bytecode_address
            );
            return None;
        }

        tracing::info!("running call in PVM {:#?}", call);

        let max_gas =
            <<Runtime as Config>::EthGasEncoder as GasEncoder<BalanceOf<Runtime>>>::encode(
                Default::default(),
                Weight::MAX,
                1u128 << 99,
            );
        let gas_limit = sp_core::U256::from(call.gas_limit).min(max_gas);

        let res = ctx.revive_test_externalities.lock().unwrap().execute_with(|| {
            let origin = OriginFor::<Runtime>::signed(AccountId::to_fallback_account_id(
                &H160::from_slice(call.caller.as_slice()),
            ));
            let evm_value = sp_core::U256::from_little_endian(&call.call_value().as_le_bytes());

            let (gas_limit, storage_deposit_limit) =
                <<Runtime as Config>::EthGasEncoder as GasEncoder<BalanceOf<Runtime>>>::decode(
                    gas_limit,
                )
                .expect("gas limit is valid");
            let storage_deposit_limit = DepositLimit::Balance(storage_deposit_limit);
            let target = H160::from_slice(call.target_address.as_slice());

            Pallet::<Runtime>::bare_call(
                origin,
                target,
                evm_value,
                gas_limit,
                storage_deposit_limit,
                call.input.to_vec(),
            )
        });

        let mut gas = Gas::new(call.gas_limit);
        let gas_used =
            <<Runtime as Config>::EthGasEncoder as GasEncoder<BalanceOf<Runtime>>>::encode(
                gas_limit,
                res.gas_required,
                res.storage_deposit.charge_or_zero(),
            );
        match res.result {
            Ok(result) => {
                let _ = gas.record_cost(gas_used.as_u64());

                let outcome = if result.did_revert() {
                    CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Revert,
                            output: result.data.into(),
                            gas,
                        },
                        memory_offset: call.return_memory_offset.clone(),
                    }
                } else {
                    CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Return,
                            output: result.data.into(),
                            gas,
                        },
                        memory_offset: call.return_memory_offset.clone(),
                    }
                };

                Some(outcome)
            }
            Err(e) => {
                tracing::error!("Contract call failed: {e:#?}");
                Some(CallOutcome {
                    result: InterpreterResult {
                        result: InstructionResult::Revert,
                        output: Bytes::from_iter(
                            format!("Contract call failed: {e:#?}").as_bytes(),
                        ),
                        gas,
                    },
                    memory_offset: call.return_memory_offset.clone(),
                })
            }
        }
    }
}

fn get_context_ref_mut(
    ctx: &mut dyn CheatcodeInspectorStrategyContext,
) -> &mut PvmCheatcodeInspectorStrategyContext {
    ctx.as_any_mut().downcast_mut().expect("expected PvmCheatcodeInspectorStrategyContext")
}
