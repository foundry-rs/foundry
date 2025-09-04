use std::sync::{Arc, Mutex};

use alloy_primitives::{Address, U256};
use foundry_cheatcodes::CheatcodeInspectorStrategy;
use foundry_compilers::{
    compilers::resolc::dual_compiled_contracts::DualCompiledContracts, ProjectCompileOutput,
};
use foundry_evm::{
    backend::BackendStrategy,
    executors::{
        strategy::ExecutorStrategyExt, EvmExecutorStrategyRunner, ExecutorStrategyContext,
        ExecutorStrategyRunner,
    },
};
use polkadot_sdk::{
    frame_support::traits::{fungible::Mutate, Currency},
    pallet_balances,
    pallet_revive::{AddressMapper, BalanceOf, BalanceWithDust},
    sp_core::{self, H160},
    sp_io,
};
use revive_env::{AccountId, ExtBuilder, Runtime, System};
use revm::primitives::{EnvWithHandlerCfg, ResultAndState};

use crate::{
    backend::{get_backend_ref, ReviveBackendStrategyBuilder, ReviveInspectContext},
    cheatcodes::PvmCheatcodeInspectorStrategyBuilder,
    executor::context::ReviveExecutorStrategyContext,
};

/// Defines the [ExecutorStrategyRunner] strategy for Revive.
#[derive(Debug, Default, Clone)]
pub struct ReviveExecutorStrategyRunner {
    pub revive_test_externalities: Arc<Mutex<sp_io::TestExternalities>>,
}

impl ReviveExecutorStrategyRunner {
    pub fn new() -> Self {
        Self {
            revive_test_externalities: Arc::new(Mutex::new(
                ExtBuilder::default()
                    .balance_genesis_config(vec![(H160::from_low_u64_be(1), 1000)])
                    .build(),
            )),
        }
    }
}

impl ExecutorStrategyRunner for ReviveExecutorStrategyRunner {
    fn new_backend_strategy(&self, _ctx: &dyn ExecutorStrategyContext) -> BackendStrategy {
        BackendStrategy::new_revive(self.revive_test_externalities.clone())
    }

    fn new_cheatcodes_strategy(
        &self,
        ctx: &dyn ExecutorStrategyContext,
    ) -> foundry_cheatcodes::CheatcodesStrategy {
        let ctx = get_context_ref(ctx);
        CheatcodeInspectorStrategy::new_pvm(
            self.revive_test_externalities.clone(),
            ctx.dual_compiled_contracts.clone(),
            ctx.resolc_startup,
        )
    }

    /// Sets the balance of an account.
    ///
    /// Amount should be in the range of [0, u128::MAX] despite the type
    /// because Ethereum balances are u256 while Polkadot balances are u128.
    fn set_balance(
        &self,
        executor: &mut foundry_evm::executors::Executor,
        address: Address,
        amount: U256,
    ) -> foundry_evm::backend::BackendResult<()> {
        let amount_pvm =
            sp_core::U256::from_little_endian(&amount.as_le_bytes()).min(u128::MAX.into());
        let balance_native =
            BalanceWithDust::<BalanceOf<Runtime>>::from_value::<Runtime>(amount_pvm).unwrap();

        EvmExecutorStrategyRunner.set_balance(executor, address, amount)?;

        let backend = get_backend_ref(executor.backend().strategy.context.as_ref());
        let mut ext = backend.revive_test_externalities.lock().unwrap();
        let min_balance = pallet_balances::Pallet::<Runtime>::minimum_balance();

        ext.execute_with(|| {
            pallet_balances::Pallet::<Runtime>::set_balance(
                &AccountId::to_fallback_account_id(&H160::from_slice(address.as_slice())),
                balance_native.into_rounded_balance().saturating_add(min_balance),
            );
        });
        Ok(())
    }

    fn get_balance(
        &self,
        executor: &foundry_evm::executors::Executor,
        address: Address,
    ) -> foundry_evm::backend::BackendResult<U256> {
        let evm_balance = EvmExecutorStrategyRunner.get_balance(executor, address)?;

        Ok(evm_balance)
    }

    fn set_nonce(
        &self,
        executor: &mut foundry_evm::executors::Executor,
        address: Address,
        nonce: u64,
    ) -> foundry_evm::backend::BackendResult<()> {
        EvmExecutorStrategyRunner.set_nonce(executor, address, nonce)?;
        let backend = get_backend_ref(executor.backend().strategy.context.as_ref());
        let mut ext = backend.revive_test_externalities.lock().unwrap();
        ext.execute_with(|| {
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
        });
        Ok(())
    }

    fn get_nonce(
        &self,
        executor: &foundry_evm::executors::Executor,
        address: Address,
    ) -> foundry_evm::backend::BackendResult<u64> {
        let evm_nonce = EvmExecutorStrategyRunner.get_nonce(executor, address)?;
        let backend = get_backend_ref(executor.backend().strategy.context.as_ref());
        let mut ext = backend.revive_test_externalities.lock().unwrap();
        let revive_nonce = ext.execute_with(|| {
            System::account_nonce(AccountId::to_fallback_account_id(&H160::from_slice(
                address.as_slice(),
            )))
        });

        assert_eq!(evm_nonce, revive_nonce as u64);
        Ok(evm_nonce)
    }

    fn call(
        &self,
        ctx: &dyn ExecutorStrategyContext,
        backend: &mut foundry_evm::backend::CowBackend<'_>,
        env: &mut EnvWithHandlerCfg,
        executor_env: &EnvWithHandlerCfg,
        inspector: &mut foundry_evm::inspectors::InspectorStack,
    ) -> eyre::Result<ResultAndState> {
        let ctx = get_context_ref(ctx);
        if ctx.wip_in_pvm {
            backend.inspect(env, inspector, Box::new(ReviveInspectContext))
        } else {
            EvmExecutorStrategyRunner.call(ctx, backend, env, executor_env, inspector)
        }
    }

    fn transact(
        &self,
        ctx: &mut dyn ExecutorStrategyContext,
        backend: &mut foundry_evm::backend::Backend,
        env: &mut EnvWithHandlerCfg,
        executor_env: &EnvWithHandlerCfg,
        inspector: &mut foundry_evm::inspectors::InspectorStack,
    ) -> eyre::Result<ResultAndState> {
        let ctx = get_context_ref_mut(ctx);
        if ctx.wip_in_pvm {
            backend.inspect(env, inspector, Box::new(ReviveInspectContext))
        } else {
            EvmExecutorStrategyRunner.transact(ctx, backend, env, executor_env, inspector)
        }
    }
}

fn get_context_ref(ctx: &dyn ExecutorStrategyContext) -> &ReviveExecutorStrategyContext {
    ctx.as_any_ref().downcast_ref().expect("expected ReviveExecutorStrategyContext")
}

fn get_context_ref_mut(
    ctx: &mut dyn ExecutorStrategyContext,
) -> &mut ReviveExecutorStrategyContext {
    ctx.as_any_mut().downcast_mut().expect("expected ReviveExecutorStrategyContext")
}

impl ExecutorStrategyExt for ReviveExecutorStrategyRunner {
    fn revive_set_dual_compiled_contracts(
        &self,
        ctx: &mut dyn ExecutorStrategyContext,
        dual_compiled_contracts: DualCompiledContracts,
    ) {
        let ctx = get_context_ref_mut(ctx);
        ctx.dual_compiled_contracts = dual_compiled_contracts;
    }

    fn revive_set_compilation_output(
        &self,
        ctx: &mut dyn ExecutorStrategyContext,
        output: ProjectCompileOutput,
    ) {
        let ctx = get_context_ref_mut(ctx);
        ctx.compilation_output.replace(output);
    }
}
