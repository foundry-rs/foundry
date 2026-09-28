//! Ethereum EVM construction.

use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_primitives::{Address, Bytes, KECCAK256_EMPTY, TxKind, U256};
use evm2::{
    Evm, ExecutionConfig, Inspector, NoopInspector, Precompiles, SpecId, TxResult,
    ethereum::{TxEnvelope, ethereum_tx_registry},
    evm::{
        Database, Db, DynDatabase, EmptyDB,
        registry::{HandlerError, HandlerResult},
    },
};
use foundry_cheatcodes::{CheatsConfig, ethereum::CheatcodeAccessMode};
use foundry_evm_core::{
    constants::{
        DEFAULT_CREATE2_DEPLOYER, DEFAULT_CREATE2_DEPLOYER_CODE, DEFAULT_CREATE2_DEPLOYER_DEPLOYER,
    },
    ethereum::{EthereumEnv, FoundryEvmTypes, LocalState},
};
use std::sync::Arc;

mod inspector;
pub use inspector::EthereumInspectorStack;

/// Constructs the Ethereum execution host used by Foundry.
#[derive(Clone, Copy, Debug, Default)]
pub struct EthereumFactory;

impl EthereumFactory {
    /// Creates an EVM with Ethereum transaction handlers and precompiles for `env.spec`.
    pub fn create<'db>(
        self,
        env: EthereumEnv,
        database: impl DynDatabase + 'db,
    ) -> Evm<'db, FoundryEvmTypes> {
        Evm::new_with_execution_config(
            ExecutionConfig::for_spec_and_version(env.spec, env.version),
            env.spec,
            env.block,
            ethereum_tx_registry(env.spec),
            database,
            Precompiles::base(env.spec),
        )
    }
}

/// Ethereum execution with copy-on-write accepted state and inspector observations.
#[derive(Clone, Debug)]
pub struct EthereumExecutor<D: Database + Clone = EmptyDB, I = NoopInspector> {
    env: EthereumEnv,
    state: LocalState<D>,
    inspector: I,
}

impl<D: Database + Clone + 'static> EthereumExecutor<D, NoopInspector> {
    /// Creates an executor over local Ethereum state.
    pub fn new(env: EthereumEnv, state: LocalState<D>) -> Self {
        Self { env, state, inspector: NoopInspector::default() }
    }
}

impl<D: Database + Clone + 'static> EthereumExecutor<D, EthereumInspectorStack> {
    /// Creates an executor with Foundry cheatcodes and observation hooks.
    pub fn new_foundry(
        env: EthereumEnv,
        mut state: LocalState<D>,
        config: Arc<CheatsConfig>,
        access_mode: CheatcodeAccessMode,
    ) -> Self {
        let inspector = EthereumInspectorStack::new(config, access_mode);
        inspector.install(&mut state);
        Self { env, state, inspector }
    }
}

impl<D: Database + Clone + 'static, I: Inspector<FoundryEvmTypes> + Clone> EthereumExecutor<D, I> {
    /// Creates an executor that retains inspector observations across accepted transactions.
    pub const fn with_inspector(env: EthereumEnv, state: LocalState<D>, inspector: I) -> Self {
        Self { env, state, inspector }
    }

    /// Returns retained inspector observations.
    pub const fn inspector(&self) -> &I {
        &self.inspector
    }

    /// Returns mutable inspector state and observations.
    pub const fn inspector_mut(&mut self) -> &mut I {
        &mut self.inspector
    }

    /// Returns the environment used for the next transaction.
    pub const fn env(&self) -> &EthereumEnv {
        &self.env
    }

    /// Returns mutable environment for the next transaction.
    pub const fn env_mut(&mut self) -> &mut EthereumEnv {
        &mut self.env
    }

    /// Returns the accepted state.
    pub const fn state(&self) -> &LocalState<D> {
        &self.state
    }

    /// Returns mutable accepted state, cloning it if shared by another executor.
    pub const fn state_mut(&mut self) -> &mut LocalState<D> {
        &mut self.state
    }

    /// Executes a transaction without accepting its state changes.
    pub fn call(&self, tx: &Recovered<TxEnvelope>) -> HandlerResult<TxResult> {
        self.inspect(tx).map(|(result, _)| result)
    }

    /// Executes without accepting state, returning this execution's inspector observations.
    pub fn inspect(&self, tx: &Recovered<TxEnvelope>) -> HandlerResult<(TxResult, I)> {
        self.inspect_with_env(self.env, tx, false)
    }

    /// Simulates a Foundry call without accepting its state changes.
    pub fn call_raw(
        &self,
        caller: Address,
        target: Address,
        input: Bytes,
        value: U256,
    ) -> HandlerResult<TxResult> {
        self.inspect_raw(caller, target, input, value).map(|(result, _)| result)
    }

    /// Simulates a Foundry call and returns its inspector observations.
    pub fn inspect_raw(
        &self,
        caller: Address,
        target: Address,
        input: Bytes,
        value: U256,
    ) -> HandlerResult<(TxResult, I)> {
        let tx = self.synthetic_tx(caller, TxKind::Call(target), input, value)?;
        self.inspect_with_env(self.synthetic_env(), &tx, true)
    }

    /// Executes a Foundry call and accepts its state changes.
    pub fn transact_raw(
        &mut self,
        caller: Address,
        target: Address,
        input: Bytes,
        value: U256,
    ) -> HandlerResult<TxResult> {
        let tx = self.synthetic_tx(caller, TxKind::Call(target), input, value)?;
        self.transact_with_env(self.synthetic_env(), &tx, true)
    }

    /// Deploys a contract through a Foundry synthetic transaction.
    pub fn deploy(&mut self, caller: Address, code: Bytes, value: U256) -> HandlerResult<TxResult> {
        let tx = self.synthetic_tx(caller, TxKind::Create, code, value)?;
        self.transact_with_env(self.synthetic_env(), &tx, true)
    }

    /// Installs the canonical local CREATE2 factory if it has no code yet.
    pub fn deploy_create2_deployer(&mut self) -> eyre::Result<()> {
        let installed = Database::get_account(&mut self.state, &DEFAULT_CREATE2_DEPLOYER)?
            .is_some_and(|account| {
                !account.code_hash.is_zero() && account.code_hash != KECCAK256_EMPTY
            });
        if installed {
            return Ok(());
        }

        let creator = DEFAULT_CREATE2_DEPLOYER_DEPLOYER;
        let balance = Database::get_account(&mut self.state, &creator)?
            .map_or(U256::ZERO, |account| account.balance);
        self.state.set_balance(creator, U256::MAX)?;
        let deployed = self.deploy(creator, DEFAULT_CREATE2_DEPLOYER_CODE.into(), U256::ZERO);
        self.state.set_balance(creator, balance)?;
        let deployed = deployed?;
        eyre::ensure!(
            deployed.status && deployed.created_address == Some(DEFAULT_CREATE2_DEPLOYER),
            "CREATE2 factory deployment failed: {:?}",
            deployed.stop
        );
        Ok(())
    }

    fn inspect_with_env(
        &self,
        env: EthereumEnv,
        tx: &Recovered<TxEnvelope>,
        synthetic: bool,
    ) -> HandlerResult<(TxResult, I)> {
        let mut state = self.state.clone();
        let mut inspector = self.inspector.clone();
        let result = {
            let mut evm = EthereumFactory.create(env, Db::new(&mut state));
            evm.ext_mut().transaction_origin = Some(tx.signer());
            if synthetic {
                evm.ext_mut().basefee_override = Some(self.env.block.basefee);
                evm.ext_mut().gas_price_override = Some(self.env.gas_price);
            }
            evm.set_inspector(&mut inspector);
            evm.transact(tx)?.discard()
        };
        Ok((result, inspector))
    }

    /// Executes and accepts a transaction's state changes.
    pub fn transact(&mut self, tx: &Recovered<TxEnvelope>) -> HandlerResult<TxResult> {
        self.transact_with_env(self.env, tx, false)
    }

    fn transact_with_env(
        &mut self,
        env: EthereumEnv,
        tx: &Recovered<TxEnvelope>,
        synthetic: bool,
    ) -> HandlerResult<TxResult> {
        let mut inspector = self.inspector.clone();
        let (outcome, mut block, basefee_override, gas_price_override) = {
            let mut evm = EthereumFactory.create(env, Db::new(&mut self.state));
            evm.ext_mut().transaction_origin = Some(tx.signer());
            if synthetic {
                evm.ext_mut().basefee_override = Some(self.env.block.basefee);
                evm.ext_mut().gas_price_override = Some(self.env.gas_price);
            }
            evm.set_inspector(&mut inspector);
            let outcome = evm.transact(tx)?.detach();
            (outcome, *evm.block(), evm.ext().basefee_override, evm.ext().gas_price_override)
        };
        if let Some(basefee) = basefee_override {
            block.basefee = basefee;
        }
        self.state.commit(&outcome.pending_state);
        self.env.block = block;
        if let Some(gas_price) = gas_price_override {
            self.env.gas_price = gas_price;
        }
        self.inspector = inspector;
        Ok(outcome.result)
    }

    const fn synthetic_env(&self) -> EthereumEnv {
        let mut env = self.env;
        env.block.basefee = U256::ZERO;
        env
    }

    fn synthetic_tx(
        &self,
        caller: Address,
        to: TxKind,
        input: Bytes,
        value: U256,
    ) -> HandlerResult<Recovered<TxEnvelope>> {
        let mut state = self.state.clone();
        let nonce = Database::get_account(&mut state, &caller)
            .map_err(HandlerError::External)?
            .map_or(0, |account| account.nonce);
        Ok(Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                chain_id: (self.env.spec >= SpecId::SPURIOUS_DRAGON)
                    .then_some(self.env.version.chain_id),
                nonce,
                gas_limit: self.env.block.gas_limit.saturating_to(),
                to,
                value,
                input,
                ..Default::default()
            }),
            caller,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::SolCall;
    use evm2::{bytecode::Bytecode, env::BlockEnvExt, evm::AccountInfo, interpreter::Interpreter};
    use foundry_cheatcodes::{Error, Vm};
    use foundry_compilers::artifacts::EvmVersion;
    use foundry_evm_core::{
        constants::CHEATCODE_ADDRESS,
        ethereum::{ForkState, fork_db},
        opts::EvmOpts,
    };
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };
    use tiny_http::{Response, Server};

    #[test]
    fn executor_discards_calls_and_commits_copy_on_write_transactions() {
        let config =
            foundry_config::Config { evm_version: EvmVersion::Cancun, ..Default::default() };
        let caller = Address::with_last_byte(0xa);
        let recipient = Address::with_last_byte(0xb);
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &recipient,
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(&[
                0x60, 0x01, 0x5f, 0x55, 0x46, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3,
            ]))),
        );
        let mut opts = EvmOpts::default();
        opts.env.chain_id = Some(31_337);
        opts.env.gas_limit = foundry_config::GasLimit(30_000_000);
        opts.memory_limit = 1_000_000;
        let env = EthereumEnv::local_from_config(&config, &opts).unwrap();
        assert_eq!(env.spec, SpecId::CANCUN);
        let mut executor = EthereumExecutor::new(env, state);
        let tx = Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
                to: TxKind::Call(recipient),
                ..Default::default()
            }),
            caller,
        );

        let call = executor.call(&tx).unwrap();
        assert!(call.status);
        assert_eq!(U256::from_be_slice(&call.output), U256::from(31_337));
        assert!(!executor.state().database().cache.accounts.contains_key(&caller));
        assert!(!executor.state().database().cache.storage.contains_key(&recipient));

        let transaction = executor.transact(&tx).unwrap();
        assert!(transaction.status);
        assert_eq!(
            executor.state().database().cache.storage[&recipient].slots[&U256::ZERO],
            U256::ONE
        );
        let snapshot = executor.clone();
        assert!(executor.transact(&tx).unwrap().status);
        assert_eq!(executor.state().database().cache.accounts[&caller].as_ref().unwrap().nonce, 2);
        assert_eq!(snapshot.state().database().cache.accounts[&caller].as_ref().unwrap().nonce, 1);
    }

    #[test]
    fn reverted_transaction_commits_nonce_without_storage() {
        let caller = Address::with_last_byte(0xa);
        let recipient = Address::with_last_byte(0xb);
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &recipient,
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(&[
                0x60, 0x01, 0x5f, 0x55, 0x5f, 0x5f, 0xfd,
            ]))),
        );
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let mut executor = EthereumExecutor::new(env, state);
        let tx = Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
                to: TxKind::Call(recipient),
                ..Default::default()
            }),
            caller,
        );

        assert!(!executor.transact(&tx).unwrap().status);
        assert_eq!(executor.state().database().cache.accounts[&caller].as_ref().unwrap().nonce, 1);
        assert_eq!(
            executor.state().database().cache.storage[&recipient]
                .slots
                .get(&U256::ZERO)
                .copied()
                .unwrap_or_default(),
            U256::ZERO
        );
    }

    #[derive(Clone, Default)]
    struct StepCounter(usize);

    impl Inspector<FoundryEvmTypes> for StepCounter {
        fn step(&mut self, _interp: &mut Interpreter<'_, '_, FoundryEvmTypes>) {
            self.0 += 1;
        }
    }

    #[test]
    fn inspector_observations_follow_transaction_acceptance() {
        let recipient = Address::with_last_byte(0xb);
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &recipient,
            AccountInfo::default()
                .with_code(Bytecode::new_legacy(Bytes::from_static(&[0x60, 0x00]))),
        );
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let mut executor = EthereumExecutor::with_inspector(env, state, StepCounter::default());
        let tx = Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
                to: TxKind::Call(recipient),
                ..Default::default()
            }),
            Address::with_last_byte(0xa),
        );

        let (_, observed) = executor.inspect(&tx).unwrap();
        assert!(observed.0 > 0);
        assert_eq!(executor.inspector().0, 0);
        assert!(executor.transact(&tx).unwrap().status);
        assert_eq!(executor.inspector().0, observed.0);
    }

    #[test]
    fn origin_opcode_reads_foundry_execution_context() {
        let recipient = Address::with_last_byte(0xb);
        let origin = Address::with_last_byte(0xc);
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &recipient,
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(&[
                0x32, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3,
            ]))),
        );
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let tx = Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
                to: TxKind::Call(recipient),
                ..Default::default()
            }),
            Address::with_last_byte(0xa),
        );
        let mut evm = EthereumFactory.create(env, Db::new(&mut state));
        evm.ext_mut().origin_override = Some(origin);

        let result = evm.transact(&tx).unwrap().discard();
        assert!(result.status);
        assert_eq!(U256::from_be_slice(&result.output), U256::from_be_slice(origin.as_slice()));
    }

    #[test]
    fn synthetic_deploy_and_call_preserve_observed_fees() {
        let caller = Address::with_last_byte(0xa);
        let fee_code =
            Bytes::from_static(&[0x48, 0x5f, 0x52, 0x3a, 0x60, 0x20, 0x52, 0x60, 0x40, 0x5f, 0xf3]);
        let mut initcode = vec![
            0x60,
            fee_code.len() as u8,
            0x60,
            12,
            0x60,
            0,
            0x39,
            0x60,
            fee_code.len() as u8,
            0x60,
            0,
            0xf3,
        ];
        initcode.extend_from_slice(&fee_code);
        let mut env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt {
                basefee: U256::from(7),
                gas_limit: U256::from(30_000_000),
                ..Default::default()
            },
        );
        env.gas_price = U256::from(5);
        let mut executor = EthereumExecutor::new_foundry(
            env,
            LocalState::default(),
            Arc::default(),
            CheatcodeAccessMode::Local,
        );
        let deployed = executor.deploy(caller, initcode.into(), U256::ZERO).unwrap();
        assert!(deployed.status);
        let contract = deployed.created_address.unwrap();
        let observed = executor.call_raw(caller, contract, Bytes::new(), U256::ZERO).unwrap();
        assert_eq!(U256::from_be_slice(&observed.output[..32]), U256::from(7));
        assert_eq!(U256::from_be_slice(&observed.output[32..]), U256::from(5));
        assert_eq!(executor.env().block.basefee, U256::from(7));

        let fee = Vm::feeCall { newBasefee: U256::from(11) }.abi_encode().into();
        assert!(executor.transact_raw(caller, CHEATCODE_ADDRESS, fee, U256::ZERO).unwrap().status);
        assert_eq!(executor.env().block.basefee, U256::from(11));
        let price = Vm::txGasPriceCall { newGasPrice: U256::from(13) }.abi_encode().into();
        assert!(
            executor.transact_raw(caller, CHEATCODE_ADDRESS, price, U256::ZERO).unwrap().status
        );
        assert_eq!(executor.env().gas_price, U256::from(13));
        let observed = executor.call_raw(caller, contract, Bytes::new(), U256::ZERO).unwrap();
        assert_eq!(U256::from_be_slice(&observed.output[..32]), U256::from(11));
        assert_eq!(U256::from_be_slice(&observed.output[32..]), U256::from(13));
    }

    #[test]
    fn gas_price_cheatcode_updates_the_active_frame() {
        let caller = Address::with_last_byte(0xa);
        let contract = Address::with_last_byte(0xb);
        let calldata = Vm::txGasPriceCall { newGasPrice: U256::from(13) }.abi_encode();
        let mut code = cheatcode_calling_contract_code(&calldata);
        code.truncate(code.len() - calldata.len() - 1);
        let continuation = [0x50, 0x3a, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3];
        code.extend_from_slice(&continuation);
        code[3] += continuation.len() as u8 - 1;
        code.extend_from_slice(&calldata);
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &contract,
            AccountInfo::default().with_code(Bytecode::new_legacy(code.into())),
        );
        let mut env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt {
                basefee: U256::from(7),
                gas_limit: U256::from(30_000_000),
                ..Default::default()
            },
        );
        env.gas_price = U256::from(5);
        let mut executor =
            EthereumExecutor::new_foundry(env, state, Arc::default(), CheatcodeAccessMode::Local);
        let observed = executor.call_raw(caller, contract, Bytes::new(), U256::ZERO).unwrap();
        assert_eq!(U256::from_be_slice(&observed.output), U256::from(13));
        assert_eq!(executor.env().gas_price, U256::from(5));

        let observed = executor.transact_raw(caller, contract, Bytes::new(), U256::ZERO).unwrap();
        assert_eq!(U256::from_be_slice(&observed.output), U256::from(13));
        assert_eq!(executor.env().gas_price, U256::from(13));
    }

    #[test]
    fn prank_changes_one_call_while_start_prank_changes_later_calls() {
        let caller = Address::with_last_byte(0xa);
        let contract = Address::with_last_byte(0xb);
        let target = Address::with_last_byte(0xc);
        let replacement = Address::with_last_byte(0xd);
        let origin = Address::with_last_byte(0xe);
        let target_code = Bytecode::new_legacy(Bytes::from_static(&[
            0x33, 0x5f, 0x52, 0x32, 0x60, 0x20, 0x52, 0x60, 0x40, 0x5f, 0xf3,
        ]));
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        for (calldata, persistent) in [
            (Vm::prank_1Call { msgSender: replacement, txOrigin: origin }.abi_encode(), false),
            (Vm::startPrank_1Call { msgSender: replacement, txOrigin: origin }.abi_encode(), true),
        ] {
            let mut state = LocalState::default();
            state.database_mut().insert_account_info(
                &contract,
                AccountInfo { balance: U256::from(1), ..Default::default() }.with_code(
                    Bytecode::new_legacy(prank_calling_contract_code(&calldata, target).into()),
                ),
            );
            state.database_mut().insert_account_info(
                &replacement,
                AccountInfo { balance: U256::from(2), ..Default::default() },
            );
            state.database_mut().insert_account_info(
                &target,
                AccountInfo::default().with_code(target_code.clone()),
            );
            let executor = EthereumExecutor::new_foundry(
                env,
                state,
                Arc::default(),
                CheatcodeAccessMode::Local,
            );
            let result = executor.call_raw(caller, contract, Bytes::new(), U256::ZERO).unwrap();
            assert!(result.status);
            assert_eq!(result.output.len(), 128);
            assert_eq!(&result.output[12..32], replacement.as_slice());
            assert_eq!(&result.output[44..64], origin.as_slice());
            let second_caller = if persistent { replacement } else { contract };
            let second_origin = if persistent { origin } else { caller };
            assert_eq!(&result.output[76..96], second_caller.as_slice());
            assert_eq!(&result.output[108..128], second_origin.as_slice());
        }

        let executor = EthereumExecutor::new_foundry(
            env,
            LocalState::default(),
            Arc::default(),
            CheatcodeAccessMode::Local,
        );
        let result = executor
            .call_raw(
                caller,
                CHEATCODE_ADDRESS,
                Vm::prank_0Call { msgSender: replacement }.abi_encode().into(),
                U256::ZERO,
            )
            .unwrap();
        assert!(!result.status);
        assert_eq!(
            result.output,
            Error::encode("top-level prank is unsupported in evm2 execution")
        );
    }

    #[test]
    fn delegate_prank_changes_sender_and_storage_context() {
        let caller = Address::with_last_byte(0xa);
        let contract = Address::with_last_byte(0xb);
        let implementation = Address::with_last_byte(0xc);
        let proxy = Address::with_last_byte(0xd);
        let origin = Address::with_last_byte(0xe);
        let calldata =
            Vm::prank_3Call { msgSender: proxy, txOrigin: origin, delegateCall: true }.abi_encode();
        let mut code = cheatcode_calling_contract_code(&calldata);
        code.truncate(code.len() - calldata.len() - 1);
        code.extend_from_slice(&[0x50, 0x60, 0x40, 0x5f, 0x5f, 0x5f, 0x73]);
        code.extend_from_slice(implementation.as_slice());
        code.extend_from_slice(&[0x61, 0xff, 0xff, 0xf4, 0x50, 0x60, 0x40, 0x5f, 0xf3]);
        code[3] = code.len() as u8;
        code.extend_from_slice(&calldata);
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &contract,
            AccountInfo::default().with_code(Bytecode::new_legacy(code.into())),
        );
        state.database_mut().insert_account_info(
            &implementation,
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(&[
                0x60, 0x2a, 0x5f, 0x55, 0x33, 0x5f, 0x52, 0x32, 0x60, 0x20, 0x52, 0x60, 0x40, 0x5f,
                0xf3,
            ]))),
        );
        state.database_mut().insert_account_info(
            &proxy,
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(&[0x00]))),
        );
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let mut executor =
            EthereumExecutor::new_foundry(env, state, Arc::default(), CheatcodeAccessMode::Local);
        let result = executor.transact_raw(caller, contract, Bytes::new(), U256::ZERO).unwrap();
        assert!(result.status, "{result:?}");
        assert_eq!(&result.output[12..32], proxy.as_slice());
        assert_eq!(&result.output[44..64], origin.as_slice());
        assert_eq!(
            Database::get_storage(&mut executor.state().clone(), &proxy, &U256::ZERO).unwrap(),
            U256::from(42)
        );
        assert_eq!(
            Database::get_storage(&mut executor.state().clone(), &contract, &U256::ZERO).unwrap(),
            U256::ZERO
        );
    }

    #[test]
    fn prank_changes_create_and_create2_deployer() {
        let caller = Address::with_last_byte(0xa);
        let contract = Address::with_last_byte(0xb);
        let replacement = Address::with_last_byte(0xc);
        let origin = Address::with_last_byte(0xd);
        let initcode = [0x32, 0x5f, 0x55, 0x5f, 0x5f, 0xf3];
        let calldata = Vm::prank_1Call { msgSender: replacement, txOrigin: origin }.abi_encode();
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        for create2 in [false, true] {
            let mut code = cheatcode_calling_contract_code(&calldata);
            code.truncate(code.len() - calldata.len() - 1);
            code.push(0x50);
            if create2 {
                code.push(0x5f); // CREATE2 salt.
            }
            code.extend_from_slice(&[0x60, initcode.len() as u8, 0x60, 0, 0x5f, 0x39]);
            let initcode_offset = code.len() - 3;
            code.extend_from_slice(&[0x60, initcode.len() as u8, 0x5f, 0x5f]);
            code.push(if create2 { 0xf5 } else { 0xf0 });
            code.extend_from_slice(&[0x5f, 0x52, 0x32, 0x60, 0x20, 0x52, 0x60, 0x40, 0x5f, 0xf3]);
            code[3] = code.len() as u8;
            code[initcode_offset] = (code.len() + calldata.len()) as u8;
            code.extend_from_slice(&calldata);
            code.extend_from_slice(&initcode);

            let mut state = LocalState::default();
            state.database_mut().insert_account_info(
                &contract,
                AccountInfo::default().with_code(Bytecode::new_legacy(code.into())),
            );
            let mut executor = EthereumExecutor::new_foundry(
                env,
                state,
                Arc::default(),
                CheatcodeAccessMode::Local,
            );
            let result = executor.transact_raw(caller, contract, Bytes::new(), U256::ZERO).unwrap();
            assert!(result.status, "{result:?}");
            let expected = if create2 {
                replacement
                    .create2(alloy_primitives::B256::ZERO, alloy_primitives::keccak256(initcode))
            } else {
                replacement.create(0)
            };
            assert_eq!(result.output.len(), 64);
            assert_eq!(&result.output[12..32], expected.as_slice());
            assert_eq!(
                U256::from_be_slice(&result.output[32..64]),
                U256::from_be_slice(caller.as_slice())
            );
            assert!(
                Database::get_account(&mut executor.state().clone(), &expected).unwrap().is_some()
            );
            assert_eq!(
                Database::get_storage(&mut executor.state().clone(), &expected, &U256::ZERO)
                    .unwrap(),
                U256::from_be_slice(origin.as_slice())
            );
            assert_eq!(
                Database::get_account(&mut executor.state().clone(), &replacement)
                    .unwrap()
                    .unwrap()
                    .nonce,
                1
            );
        }
    }

    #[test]
    fn synthetic_call_respects_pre_eip155_transactions() {
        let caller = Address::with_last_byte(0xa);
        let contract = Address::with_last_byte(0xb);
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &contract,
            AccountInfo::default().with_code(Bytecode::new_legacy(Bytes::from_static(&[
                0x60, 0x01, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
            ]))),
        );
        let env = EthereumEnv::new(
            SpecId::HOMESTEAD,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let executor = EthereumExecutor::new(env, state);
        let result = executor.call_raw(caller, contract, Bytes::new(), U256::ZERO).unwrap();
        assert!(result.status);
        assert_eq!(U256::from_be_slice(&result.output), U256::from(1));
    }

    #[test]
    fn cheatcode_dispatch_enforces_policy_and_execution_boundaries() {
        let caller = Address::with_last_byte(0xa);
        let target = Address::with_last_byte(0xb);
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let deal = cheatcode_tx(
            caller,
            Vm::dealCall { account: target, newBalance: U256::from(7) }.abi_encode().into(),
        );
        let mut executor = EthereumExecutor::new_foundry(
            env,
            LocalState::default(),
            Arc::default(),
            CheatcodeAccessMode::Local,
        );
        assert!(executor.call(&deal).unwrap().status);
        assert!(!executor.state().database().cache.accounts.contains_key(&target));
        assert!(executor.transact(&deal).unwrap().status);
        assert_eq!(
            executor.state().database().account_info(&target).unwrap().balance,
            U256::from(7)
        );

        let mut config = CheatsConfig::default();
        config.blocked_cheatcodes.push(Vm::dealCall::SELECTOR);
        let blocked = EthereumExecutor::new_foundry(
            env,
            LocalState::default(),
            Arc::new(config),
            CheatcodeAccessMode::Local,
        );
        let blocked_result = blocked.call(&deal).unwrap();
        assert!(!blocked_result.status);
        assert_eq!(
            blocked_result.output,
            Error::encode("vm.deal: disabled during restricted execution")
        );

        let mut forked = EthereumExecutor::new_foundry(
            env,
            LocalState::default(),
            Arc::default(),
            CheatcodeAccessMode::Forked,
        );
        let denied = forked.call(&deal).unwrap();
        assert!(!denied.status);
        assert_eq!(
            denied.output,
            Error::encode(format!("vm.deal: cheatcode access denied for {caller}"))
        );
        forked.inspector_mut().cheatcodes_mut().allow_caller(caller);
        assert!(forked.transact(&deal).unwrap().status);

        let unsupported = cheatcode_tx(
            caller,
            Vm::etchCall { target, newRuntimeBytecode: Bytes::new() }.abi_encode().into(),
        );
        let unsupported_executor = EthereumExecutor::new_foundry(
            env,
            LocalState::default(),
            Arc::default(),
            CheatcodeAccessMode::Local,
        );
        let unsupported_result = unsupported_executor.call(&unsupported).unwrap();
        assert!(!unsupported_result.status);
        assert_eq!(
            unsupported_result.output,
            Error::encode("vm.etch: unsupported in evm2 execution")
        );
    }

    #[test]
    fn forked_contract_needs_its_own_cheatcode_access() {
        let caller = Address::with_last_byte(0xa);
        let contract = Address::with_last_byte(0xb);
        let target = Address::with_last_byte(0xc);
        let calldata = Vm::dealCall { account: target, newBalance: U256::from(7) }.abi_encode();
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &contract,
            AccountInfo::default()
                .with_code(Bytecode::new_legacy(cheatcode_calling_contract_code(&calldata).into())),
        );
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let tx = Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
                to: TxKind::Call(contract),
                ..Default::default()
            }),
            caller,
        );

        let mut denied = EthereumExecutor::new_foundry(
            env,
            state.clone(),
            Arc::default(),
            CheatcodeAccessMode::Forked,
        );
        denied.inspector_mut().cheatcodes_mut().allow_caller(caller);
        assert!(denied.transact(&tx).unwrap().status);
        assert!(denied.state().database().account_info(&target).is_none());

        let mut allowed =
            EthereumExecutor::new_foundry(env, state, Arc::default(), CheatcodeAccessMode::Forked);
        allowed.inspector_mut().cheatcodes_mut().allow_caller(contract);
        assert!(allowed.transact(&tx).unwrap().status);
        assert_eq!(
            allowed.state().database().account_info(&target).unwrap().balance,
            U256::from(7)
        );
    }

    #[test]
    fn state_cheatcodes_commit_and_read_journaled_state() {
        let caller = Address::with_last_byte(0xa);
        let target = Address::with_last_byte(0xb);
        let slot = U256::from(2).into();
        let value = U256::from(17).into();
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt {
                number: U256::from(8),
                timestamp: U256::from(9),
                gas_limit: U256::from(30_000_000),
                ..Default::default()
            },
        );
        let mut executor = EthereumExecutor::new_foundry(
            env,
            LocalState::default(),
            Arc::default(),
            CheatcodeAccessMode::Local,
        );
        let store = cheatcode_tx(caller, Vm::storeCall { target, slot, value }.abi_encode().into());
        assert!(executor.transact(&store).unwrap().status);
        let load =
            cheatcode_tx_at_nonce(caller, Vm::loadCall { target, slot }.abi_encode().into(), 1);
        assert_eq!(executor.call(&load).unwrap().output.as_ref(), value.as_slice());

        let set_nonce = cheatcode_tx_at_nonce(
            caller,
            Vm::setNonceCall { account: target, newNonce: 8 }.abi_encode().into(),
            1,
        );
        assert!(executor.transact(&set_nonce).unwrap().status);
        let get_nonce = cheatcode_tx_at_nonce(
            caller,
            Vm::getNonce_0Call { account: target }.abi_encode().into(),
            2,
        );
        assert_eq!(U256::from_be_slice(&executor.call(&get_nonce).unwrap().output), U256::from(8));
        let decrease = cheatcode_tx_at_nonce(
            caller,
            Vm::setNonceCall { account: target, newNonce: 7 }.abi_encode().into(),
            2,
        );
        assert!(!executor.call(&decrease).unwrap().status);
        assert_eq!(executor.state().database().account_info(&target).unwrap().nonce, 8);
        let set_nonce_unsafe = cheatcode_tx_at_nonce(
            caller,
            Vm::setNonceUnsafeCall { account: target, newNonce: 7 }.abi_encode().into(),
            2,
        );
        assert!(executor.transact(&set_nonce_unsafe).unwrap().status);
        assert_eq!(executor.state().database().account_info(&target).unwrap().nonce, 7);

        let height =
            cheatcode_tx_at_nonce(caller, Vm::getBlockNumberCall {}.abi_encode().into(), 3);
        let timestamp =
            cheatcode_tx_at_nonce(caller, Vm::getBlockTimestampCall {}.abi_encode().into(), 3);
        assert_eq!(U256::from_be_slice(&executor.call(&height).unwrap().output), U256::from(8));
        assert_eq!(U256::from_be_slice(&executor.call(&timestamp).unwrap().output), U256::from(9));

        let precompile = cheatcode_tx_at_nonce(
            caller,
            Vm::storeCall { target: Address::with_last_byte(1), slot, value }.abi_encode().into(),
            3,
        );
        let result = executor.call(&precompile).unwrap();
        assert!(!result.status);
        assert_eq!(
            result.output,
            Error::encode(format!(
                "cannot use precompile {} as an argument",
                Address::with_last_byte(1)
            ))
        );
    }

    #[test]
    fn nested_revert_rolls_back_cheatcode_storage_write() {
        let caller = Address::with_last_byte(0xa);
        let contract = Address::with_last_byte(0xb);
        let target = Address::with_last_byte(0xc);
        let slot = U256::from(2).into();
        let value = U256::from(17).into();
        let calldata = Vm::storeCall { target, slot, value }.abi_encode();
        let normal_code = cheatcode_calling_contract_code(&calldata);
        let mut state = LocalState::default();
        state.database_mut().insert_account_info(
            &contract,
            AccountInfo::default().with_code(Bytecode::new_legacy(normal_code.clone().into())),
        );
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let tx = Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
                to: TxKind::Call(contract),
                ..Default::default()
            }),
            caller,
        );
        let mut normal = EthereumExecutor::new_foundry(
            env,
            state.clone(),
            Arc::default(),
            CheatcodeAccessMode::Local,
        );
        assert!(normal.transact(&tx).unwrap().status);
        assert_eq!(
            Database::get_storage(&mut normal.state().clone(), &target, &U256::from(2)).unwrap(),
            U256::from(17)
        );

        let mut revert_code = normal_code;
        revert_code.truncate(revert_code.len() - calldata.len() - 1);
        revert_code.extend_from_slice(&[0x50, 0x5f, 0x5f, 0xfd]);
        revert_code[3] += 3;
        revert_code.extend_from_slice(&calldata);
        state.database_mut().insert_account_info(
            &contract,
            AccountInfo::default().with_code(Bytecode::new_legacy(revert_code.into())),
        );
        let mut executor =
            EthereumExecutor::new_foundry(env, state, Arc::default(), CheatcodeAccessMode::Local);
        assert!(!executor.transact(&tx).unwrap().status);
        assert!(executor.state().database().account_info(&target).is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rpc_fork_reads_do_not_commit_to_the_backing_database() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let handle = std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap() {
                    let rpc: serde_json::Value =
                        serde_json::from_reader(request.as_reader()).unwrap();
                    let result = match rpc["method"].as_str().unwrap() {
                        "eth_getAccountInfo" => serde_json::json!({
                            "balance": "0x0", "nonce": "0x0",
                            "code": "0x5f546001015f555f545f5260205ff3"
                        }),
                        "eth_getStorageAt" => serde_json::json!("0x3"),
                        other => panic!("unexpected RPC method: {other}"),
                    };
                    let response =
                        serde_json::json!({ "jsonrpc": "2.0", "id": rpc["id"], "result": result });
                    request.respond(Response::from_string(response.to_string())).unwrap();
                }
            }
        });

        let caller = Address::with_last_byte(0xa);
        let contract = Address::with_last_byte(0xb);
        let meta = fork_db::cache::BlockchainDbMeta::new(serde_json::Value::Null, endpoint.clone())
            .with_account_fetch_policy(fork_db::AccountFetchPolicy::RequireAccountInfo);
        let db = fork_db::BlockchainDb::new(meta, None);
        db.accounts().write().insert(caller, AccountInfo::default());
        let provider = EvmOpts::default().fork_provider_with_url(&endpoint).unwrap();
        let backend: fork_db::SharedBackend =
            fork_db::SharedBackend::spawn_backend(Arc::new(provider), db.clone(), None).await;
        let env = EthereumEnv::new(
            SpecId::CANCUN,
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        let mut executor = EthereumExecutor::new(env, ForkState::new(backend));
        let tx = Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
                to: TxKind::Call(contract),
                ..Default::default()
            }),
            caller,
        );

        assert_eq!(U256::from_be_slice(&executor.call(&tx).unwrap().output), U256::from(4));
        assert_eq!(U256::from_be_slice(&executor.transact(&tx).unwrap().output), U256::from(4));
        assert_eq!(
            executor.state().database().cache.storage[&contract].slots[&U256::ZERO],
            U256::from(4)
        );
        assert_eq!(db.storage().read()[&contract][&U256::ZERO], U256::from(3));

        stopped.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    fn cheatcode_tx(caller: Address, input: Bytes) -> Recovered<TxEnvelope> {
        cheatcode_tx_at_nonce(caller, input, 0)
    }

    fn cheatcode_tx_at_nonce(caller: Address, input: Bytes, nonce: u64) -> Recovered<TxEnvelope> {
        Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
                nonce,
                to: TxKind::Call(CHEATCODE_ADDRESS),
                input,
                ..Default::default()
            }),
            caller,
        )
    }

    fn cheatcode_calling_contract_code(calldata: &[u8]) -> Vec<u8> {
        let mut code = vec![
            0x60,
            calldata.len() as u8,
            0x60,
            0,
            0x60,
            0,
            0x39, // Copy calldata into memory.
            0x60,
            0,
            0x60,
            0,
            0x60,
            calldata.len() as u8,
            0x60,
            0,
            0x60,
            0,
            0x73,
        ];
        code.extend_from_slice(CHEATCODE_ADDRESS.as_slice());
        code.extend_from_slice(&[0x61, 0x27, 0x10, 0xf1, 0x00]);
        code[3] = code.len() as u8;
        code.extend_from_slice(calldata);
        code
    }

    fn prank_calling_contract_code(calldata: &[u8], target: Address) -> Vec<u8> {
        let mut code = cheatcode_calling_contract_code(calldata);
        code.truncate(code.len() - calldata.len() - 1);
        code.push(0x50);
        for offset in [0, 64] {
            code.extend_from_slice(&[0x60, 0x40, 0x60, offset, 0x5f, 0x5f, 0x60, 0x01, 0x73]);
            code.extend_from_slice(target.as_slice());
            code.extend_from_slice(&[0x61, 0x27, 0x10, 0xf1, 0x50]);
        }
        code.extend_from_slice(&[0x60, 0x80, 0x5f, 0xf3]);
        code[3] = code.len() as u8;
        code.extend_from_slice(calldata);
        code
    }
}
