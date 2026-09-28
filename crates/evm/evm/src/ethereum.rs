//! Ethereum EVM construction.

use alloy_consensus::transaction::Recovered;
use evm2::{
    Evm, ExecutionConfig, Inspector, NoopInspector, Precompiles, TxResult,
    ethereum::{TxEnvelope, ethereum_tx_registry},
    evm::{Database, Db, DynDatabase, EmptyDB, registry::HandlerResult},
};
use foundry_cheatcodes::{CheatsConfig, ethereum::CheatcodeAccessMode};
use foundry_evm_core::ethereum::{EthereumEnv, FoundryEvmTypes, LocalState};
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
        let mut state = self.state.clone();
        let mut inspector = self.inspector.clone();
        let result = {
            let mut evm = EthereumFactory.create(self.env, Db::new(&mut state));
            evm.set_inspector(&mut inspector);
            evm.transact(tx)?.discard()
        };
        Ok((result, inspector))
    }

    /// Executes and accepts a transaction's state changes.
    pub fn transact(&mut self, tx: &Recovered<TxEnvelope>) -> HandlerResult<TxResult> {
        let mut inspector = self.inspector.clone();
        let (outcome, block) = {
            let mut evm = EthereumFactory.create(self.env, Db::new(&mut self.state));
            evm.set_inspector(&mut inspector);
            let outcome = evm.transact(tx)?.detach();
            (outcome, *evm.block())
        };
        self.state.commit(&outcome.pending_state);
        self.env.block = block;
        self.inspector = inspector;
        Ok(outcome.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::TxLegacy;
    use alloy_primitives::{Address, Bytes, TxKind, U256};
    use alloy_sol_types::SolCall;
    use evm2::{
        SpecId, bytecode::Bytecode, env::BlockEnvExt, evm::AccountInfo, interpreter::Interpreter,
    };
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
        Recovered::new_unchecked(
            TxEnvelope::Legacy(TxLegacy {
                gas_limit: 100_000,
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
}
