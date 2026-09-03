//! EVM executor abstractions, which can execute calls.
//!
//! Used for running tests, scripts, and interacting with the inner backend which holds the state.

use crate::inspectors::{
    Cheatcodes, CmpOperands, EdgeCoverage, EdgeIndexMap, InspectorData, InspectorStack,
    cheatcodes::BroadcastableTransactions,
};
use alloy_dyn_abi::{DynSolValue, FunctionExt, JsonAbiExt};
use alloy_eips::eip4788::{BEACON_ROOTS_ADDRESS, SYSTEM_ADDRESS};
use alloy_evm::Evm;
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, B256, Bytes, Log, TxKind, U256, keccak256,
    map::{AddressHashMap, HashMap},
};
use alloy_sol_types::{SolCall, sol};
use eyre::WrapErr;
#[cfg(feature = "monad")]
use foundry_common::{SYSTEM_TRANSACTION_TYPE, is_known_system_sender};
#[cfg(feature = "monad")]
use foundry_evm_core::evm::{MonadEvmNetwork, try_transact_monad_system_replay};
#[cfg(feature = "monad")]
use foundry_evm_core::refresh_chain_journal;
use foundry_evm_core::{
    EvmEnv, FoundryBlock, FoundryChain, FoundryTransaction,
    backend::{
        Backend, BackendError, BackendResult, CowBackend, DatabaseError, DatabaseExt,
        GLOBAL_FAIL_SLOT,
    },
    constants::{
        CALLER, CHEATCODE_ADDRESS, CHEATCODE_CONTRACT_HASH, DEFAULT_CREATE2_DEPLOYER,
        DEFAULT_CREATE2_DEPLOYER_CODE, DEFAULT_CREATE2_DEPLOYER_DEPLOYER,
    },
    decode::{RevertDecoder, SkipReason},
    eip2935::{
        HISTORY_STORAGE_ADDRESS, HISTORY_STORAGE_CODE, history_storage_slot, history_storage_value,
        history_window_start,
    },
    evm::{
        BlockContext, ChainFor, EthEvmNetwork, EvmEnvFor, FoundryEvmFactory, FoundryEvmNetwork,
        IntoInstructionResult, SpecFor, TxEnvFor,
    },
    utils::StateChangeset,
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::ObservedCall;
use foundry_evm_networks::NetworkConfigs;
use foundry_evm_traces::{SparsedTraceArena, TraceRequirements};
use revm::{
    bytecode::Bytecode,
    context::{Block, Cfg, ContextTr, Transaction},
    context_interface::{
        cfg::gas_params::Eip2780TxInfo,
        result::{ExecutionResult, Output, ResultAndState},
        transaction::SignedAuthorization,
    },
    database::{Database, DatabaseCommit, DatabaseRef},
    interpreter::{InstructionResult, return_ok},
    primitives::hardfork::SpecId,
};
use sancov::SancovGuard;
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

mod builder;
pub use builder::ExecutorBuilder;

mod campaign;

pub mod fuzz;
pub use fuzz::FuzzedExecutor;

pub mod invariant;
pub use invariant::InvariantExecutor;

mod corpus;
mod corpus_io;
mod sancov;
mod showmap;
mod trace;

pub use corpus::{DynamicTargetCtx, StatelessReplayTarget, persist_corpus_seed};
pub use corpus_io::{
    CorpusDirEntry, canonical_replay_dirs, parse_corpus_filename, read_corpus_dir, read_corpus_tree,
};
pub use showmap::{
    InvariantReplayOptions, MinimizationReplayInput, ReplayFailure, ReplayObservation,
    ShowmapDomain, ShowmapOpts, ShowmapReplayTarget, ShowmapStats, replay_corpus_to_showmap,
    replay_sequence_for_minimization,
};
pub use trace::TracingExecutor;

const DURATION_BETWEEN_METRICS_REPORT: Duration = Duration::from_secs(5);

sol! {
    interface ITest {
        function setUp() external;
        function failed() external view returns (bool failed);

        #[derive(Default)]
        function beforeTestSetup(bytes4 testSelector) public view returns (bytes[] memory beforeTestCalldata);
    }
}

/// EVM executor.
///
/// The executor can be configured with various `revm::Inspector`s, like `Cheatcodes`.
///
/// There are multiple ways of interacting the EVM:
/// - `call`: executes a transaction, but does not persist any state changes; similar to `eth_call`,
///   where the EVM state is unchanged after the call.
/// - `transact`: executes a transaction and persists the state changes
/// - `deploy`: a special case of `transact`, specialized for persisting the state of a contract
///   deployment
/// - `setup`: a special case of `transact`, used to set up the environment for a test
#[derive(Clone, Debug)]
pub struct Executor<FEN: FoundryEvmNetwork> {
    /// The underlying `revm::Database` that contains the EVM storage.
    ///
    /// Wrapped in `Arc` for efficient cloning during parallel fuzzing. Use [`Arc::make_mut`]
    /// for copy-on-write semantics when mutation is needed.
    // Note: We do not store an EVM here, since we are really
    // only interested in the database. REVM's `EVM` is a thin
    // wrapper around spawning a new EVM on every call anyway,
    // so the performance difference should be negligible.
    backend: Arc<Backend<FEN>>,
    /// The EVM environment (block and cfg).
    evm_env: EvmEnvFor<FEN>,
    /// The transaction environment.
    tx_env: TxEnvFor<FEN>,
    /// The Revm inspector stack.
    inspector: InspectorStack<FEN>,
    /// The gas limit for calls and deployments.
    gas_limit: u64,
    /// Whether `failed()` should be called on the test contract to determine if the test failed.
    legacy_assertions: bool,
    /// Opt-in cursor for transactions simulated sequentially against one fork.
    block_context: Option<BlockContext<FEN>>,
}

#[cfg(feature = "monad")]
impl Executor<MonadEvmNetwork> {
    /// Replays Monad transactions and executes the target against one EVM instance.
    #[instrument(name = "transact_monad_block_replay", level = "debug", skip_all)]
    pub fn transact_with_monad_block_replay(
        &mut self,
        evm_env: EvmEnvFor<MonadEvmNetwork>,
        target_tx_env: TxEnvFor<MonadEvmNetwork>,
        target_chain_context: ChainFor<MonadEvmNetwork>,
        replay: Vec<(B256, TxEnvFor<MonadEvmNetwork>, ChainFor<MonadEvmNetwork>)>,
        replay_system_txes: bool,
    ) -> eyre::Result<Option<(RawCallResult<MonadEvmNetwork>, bool)>> {
        let block_number = evm_env.block_env.number();
        let mut stack = self.inspector().clone();
        let sancov_edges = stack.inner.sancov_edges;
        let sancov_trace_cmp = stack.inner.sancov_trace_cmp;
        let sancov_active = sancov_edges || sancov_trace_cmp;
        let backend = self.backend_mut();

        let (result, evm_env, tx_env, used_system_replay) = {
            let caller = target_tx_env.caller();
            backend.set_caller(caller).set_spec_id(evm_env.cfg_env.spec);
            let target_contract = match target_tx_env.kind() {
                TxKind::Call(to) => to,
                TxKind::Create => caller.create(target_tx_env.nonce()),
            };
            backend.set_test_contract(target_contract);
            let mut evm = <MonadEvmNetwork as FoundryEvmNetwork>::EvmFactory::default()
                .create_foundry_evm_with_inspector(
                    backend,
                    evm_env,
                    target_chain_context.clone(),
                    &mut stack,
                );
            evm.disable_inspector();
            for (tx_hash, tx_env, chain_context) in replay {
                evm.ctx_mut().chain = chain_context;
                refresh_chain_journal(evm.ctx_mut());
                evm.ctx_mut().cfg.disable_balance_check = true;
                let is_system = is_known_system_sender(tx_env.caller())
                    || tx_env.tx_type() == SYSTEM_TRANSACTION_TYPE;
                let result = if is_system {
                    try_transact_monad_system_replay(&mut evm, &tx_env).wrap_err_with(|| {
                        format!(
                            "Failed to replay system transaction: {tx_hash:?} in block {block_number}"
                        )
                    })?
                } else {
                    None
                };
                if let Some(result) = result {
                    evm.db_mut().commit(result.state);
                } else if !is_system || replay_system_txes {
                    let created = match tx_env.kind() {
                        TxKind::Create => Some(tx_env.caller().create(tx_env.nonce())),
                        TxKind::Call(_) => None,
                    };
                    let result = evm.transact(tx_env).wrap_err_with(|| {
                        format!(
                            "Failed to execute transaction: {tx_hash:?} in block {block_number}"
                        )
                    })?;
                    if result.result.is_success()
                        && let Some(address) = created
                    {
                        evm.db_mut().add_persistent_account(address);
                    }
                    evm.db_mut().commit(result.state);
                }
            }

            evm.enable_inspector();
            evm.ctx_mut().chain = target_chain_context;
            refresh_chain_journal(evm.ctx_mut());
            let _guard = sancov_active.then(|| SancovGuard::new(sancov_edges, sancov_trace_cmp));
            let target_is_system = is_known_system_sender(target_tx_env.caller())
                || target_tx_env.tx_type() == SYSTEM_TRANSACTION_TYPE;
            let system_result = if target_is_system {
                try_transact_monad_system_replay(&mut evm, &target_tx_env)?
            } else {
                None
            };
            let (result, used_system_replay) = if let Some(result) = system_result {
                (result, true)
            } else if target_is_system && !replay_system_txes {
                return Ok(None);
            } else {
                (evm.transact(target_tx_env.clone()).wrap_err("EVM error")?, false)
            };
            let tx_env = if used_system_replay { target_tx_env } else { evm.tx().clone() };
            let evm_env = evm.finish().1;
            (result, evm_env, tx_env, used_system_replay)
        };

        let has_state_snapshot_failure = backend.has_state_snapshot_failure();
        let fork_block_number = backend.active_fork_block_number();
        let mut result = convert_executed_result(
            evm_env,
            tx_env,
            stack,
            result,
            &*backend,
            has_state_snapshot_failure,
            fork_block_number,
        )?;
        if sancov_edges {
            SancovGuard::append_edges_into(&mut result);
        }
        if sancov_trace_cmp {
            SancovGuard::drain_cmp_into(&mut result);
        }
        self.commit(&mut result);
        Ok(Some((result, used_system_replay)))
    }
}

impl<FEN: FoundryEvmNetwork> Executor<FEN> {
    /// Creates a new `Executor` with the given arguments.
    #[inline]
    pub fn new(
        mut backend: Backend<FEN>,
        evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
        mut inspector: InspectorStack<FEN>,
        networks: NetworkConfigs,
        gas_limit: u64,
        legacy_assertions: bool,
    ) -> Self {
        inspector.networks(networks);
        backend.set_networks(networks);
        let extra_cheatcode_addresses = inspector.extra_cheatcode_addresses();
        backend.extend_persistent_accounts(extra_cheatcode_addresses.iter().copied());

        // Need to create a non-empty contract on the cheatcodes address so `extcodesize` checks
        // do not fail.
        backend.insert_account_info(
            CHEATCODE_ADDRESS,
            revm::state::AccountInfo {
                code: Some(Bytecode::new_raw(Bytes::from_static(&[0]))),
                // Also set the code hash manually so that it's not computed later.
                // The code hash value does not matter, as long as it's not zero or `KECCAK_EMPTY`.
                code_hash: CHEATCODE_CONTRACT_HASH,
                ..Default::default()
            },
        );

        for &address in extra_cheatcode_addresses {
            backend.insert_account_info(
                address,
                revm::state::AccountInfo {
                    code: Some(Bytecode::new_raw(Bytes::from_static(&[0]))),
                    code_hash: keccak256(address),
                    ..Default::default()
                },
            );
        }

        if !backend.is_in_forking_mode() && evm_env.cfg_env.spec.into() >= SpecId::PRAGUE {
            let mut account =
                backend.basic_ref(HISTORY_STORAGE_ADDRESS).unwrap_or_default().unwrap_or_default();
            account.code_hash = keccak256(&HISTORY_STORAGE_CODE);
            account.code = Some(Bytecode::new_raw(HISTORY_STORAGE_CODE.clone()));
            backend.insert_account_info(HISTORY_STORAGE_ADDRESS, account);

            let current_block = evm_env.block_env.number();
            let mut block_number = history_window_start(current_block);
            while block_number < current_block {
                let block_hash =
                    backend.block_hash(block_number.saturating_to()).unwrap_or_default();
                let slot = history_storage_slot(block_number);
                let value = history_storage_value(block_hash);
                let _ = backend.insert_account_storage(HISTORY_STORAGE_ADDRESS, slot, value);
                block_number += U256::from(1);
            }
        }

        Self {
            backend: Arc::new(backend),
            evm_env,
            tx_env,
            inspector,
            gas_limit,
            legacy_assertions,
            block_context: None,
        }
    }

    fn clone_with_backend(&self, backend: Backend<FEN>) -> Self {
        let evm_env = self.evm_env.clone();
        Self {
            backend: Arc::new(backend),
            evm_env,
            tx_env: self.tx_env.clone(),
            inspector: self.inspector().clone(),
            gas_limit: self.gas_limit,
            legacy_assertions: self.legacy_assertions,
            block_context: self.block_context.clone(),
        }
    }

    /// Returns a reference to the EVM backend.
    pub fn backend(&self) -> &Backend<FEN> {
        &self.backend
    }

    /// Returns a mutable reference to the EVM backend.
    ///
    /// Uses copy-on-write semantics: if other clones of this executor share the backend,
    /// this will clone the backend first.
    pub fn backend_mut(&mut self) -> &mut Backend<FEN> {
        Arc::make_mut(&mut self.backend)
    }

    /// Enables exact block-context progression for sequential committed transactions.
    ///
    /// This is opt-in because test and setup calls are execution phases rather than transactions
    /// that should automatically become part of one simulated block.
    pub fn enable_block_context_progression(&mut self) -> eyre::Result<()> {
        self.block_context = self.backend().block_context_for_synthetic_transaction()?;
        Ok(())
    }

    /// Advances an enabled block-context cursor to the start of the next block.
    pub fn advance_block_context(&mut self) {
        if let Some(context) = &mut self.block_context {
            context.advance_block();
        }
    }

    fn chain_context_for_synthetic_transaction(
        &self,
        tx: &TxEnvFor<FEN>,
    ) -> eyre::Result<ChainFor<FEN>> {
        self.block_context.as_ref().map_or_else(
            || self.backend().chain_context_for_synthetic_transaction(tx),
            |context| Ok(context.next_transaction(tx)),
        )
    }

    fn record_block_transaction(&mut self, tx: TxEnvFor<FEN>) {
        if let Some(context) = &mut self.block_context {
            context.record_transaction(tx);
        }
    }

    /// Returns a reference to the EVM environment (block and cfg).
    pub const fn evm_env(&self) -> &EvmEnvFor<FEN> {
        &self.evm_env
    }

    /// Returns a mutable reference to the EVM environment (block and cfg).
    pub const fn evm_env_mut(&mut self) -> &mut EvmEnvFor<FEN> {
        &mut self.evm_env
    }

    /// Returns a reference to the transaction environment.
    pub const fn tx_env(&self) -> &TxEnvFor<FEN> {
        &self.tx_env
    }

    /// Returns a mutable reference to the transaction environment.
    pub const fn tx_env_mut(&mut self) -> &mut TxEnvFor<FEN> {
        &mut self.tx_env
    }

    /// Returns a reference to the EVM inspector.
    pub const fn inspector(&self) -> &InspectorStack<FEN> {
        &self.inspector
    }

    /// Returns a mutable reference to the EVM inspector.
    pub const fn inspector_mut(&mut self) -> &mut InspectorStack<FEN> {
        &mut self.inspector
    }

    /// Returns the EVM spec.
    pub const fn spec_id(&self) -> SpecFor<FEN> {
        self.evm_env.cfg_env.spec
    }

    /// Sets the EVM spec and updates spec-dependent gas parameters.
    pub fn set_spec_id(&mut self, spec_id: SpecFor<FEN>) {
        self.evm_env.cfg_env.set_spec_and_mainnet_gas_params(spec_id);
    }

    /// Returns the gas limit for calls and deployments.
    ///
    /// This is different from the gas limit imposed by the passed in environment, as those limits
    /// are used by the EVM for certain opcodes like `gaslimit`.
    pub const fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    /// Sets the gas limit for calls and deployments.
    pub const fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_limit = gas_limit;
    }

    /// Returns whether `failed()` should be called on the test contract to determine if the test
    /// failed.
    pub const fn legacy_assertions(&self) -> bool {
        self.legacy_assertions
    }

    /// Sets whether `failed()` should be called on the test contract to determine if the test
    /// failed.
    pub const fn set_legacy_assertions(&mut self, legacy_assertions: bool) {
        self.legacy_assertions = legacy_assertions;
    }

    /// Creates the default CREATE2 Contract Deployer for local tests and scripts.
    pub fn deploy_create2_deployer(&mut self) -> eyre::Result<()> {
        trace!("deploying local create2 deployer");
        let create2_deployer_account = self
            .backend()
            .basic_ref(DEFAULT_CREATE2_DEPLOYER)?
            .ok_or_else(|| BackendError::MissingAccount(DEFAULT_CREATE2_DEPLOYER))?;

        // If the deployer is not currently deployed, deploy the default one.
        if create2_deployer_account.code.is_none_or(|code| code.is_empty()) {
            let creator = DEFAULT_CREATE2_DEPLOYER_DEPLOYER;

            // Probably 0, but just in case.
            let initial_balance = self.get_balance(creator)?;
            self.set_balance(creator, U256::MAX)?;

            let res =
                self.deploy(creator, DEFAULT_CREATE2_DEPLOYER_CODE.into(), U256::ZERO, None)?;
            trace!(create2=?res.address, "deployed local create2 deployer");

            self.set_balance(creator, initial_balance)?;
        }
        Ok(())
    }

    /// Set the balance of an account.
    pub fn set_balance(&mut self, address: Address, amount: U256) -> BackendResult<()> {
        trace!(?address, ?amount, "setting account balance");
        let mut account = self.backend().basic_ref(address)?.unwrap_or_default();
        account.balance = amount;
        self.backend_mut().insert_account_info(address, account);
        Ok(())
    }

    /// Gets the balance of an account
    pub fn get_balance(&self, address: Address) -> BackendResult<U256> {
        Ok(self.backend().basic_ref(address)?.map(|acc| acc.balance).unwrap_or_default())
    }

    /// Sets the nonce of an account without modifying the transaction environment.
    pub fn set_account_nonce(&mut self, address: Address, nonce: u64) -> BackendResult<()> {
        let mut account = self.backend().basic_ref(address)?.unwrap_or_default();
        account.nonce = nonce;
        self.backend_mut().insert_account_info(address, account);
        Ok(())
    }

    /// Sets the nonce of an account and the transaction environment.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) -> BackendResult<()> {
        self.set_account_nonce(address, nonce)?;
        self.tx_env_mut().set_nonce(nonce);
        Ok(())
    }

    /// Returns the nonce of an account.
    pub fn get_nonce(&self, address: Address) -> BackendResult<u64> {
        Ok(self.backend().basic_ref(address)?.map(|acc| acc.nonce).unwrap_or_default())
    }

    /// Set the code of an account.
    pub fn set_code(&mut self, address: Address, code: Bytecode) -> BackendResult<()> {
        let mut account = self.backend().basic_ref(address)?.unwrap_or_default();
        account.code_hash = keccak256(code.original_byte_slice());
        account.code = Some(code);
        self.backend_mut().insert_account_info(address, account);
        Ok(())
    }

    /// Set the storage of an account.
    pub fn set_storage(
        &mut self,
        address: Address,
        storage: HashMap<U256, U256>,
    ) -> BackendResult<()> {
        self.backend_mut().replace_account_storage(address, storage)?;
        Ok(())
    }

    /// Set a storage slot of an account.
    pub fn set_storage_slot(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> BackendResult<()> {
        self.backend_mut().insert_account_storage(address, slot, value)?;
        Ok(())
    }

    /// Apply prestate trace data to the executor's backend.
    ///
    /// This is used to set up the EVM state based on the prestate trace from
    /// `debug_traceTransaction`, which provides all accounts and storage slots
    /// that will be accessed during transaction execution.
    pub fn apply_prestate_trace(
        &mut self,
        prestate: std::collections::BTreeMap<Address, alloy_rpc_types::trace::geth::AccountState>,
    ) -> eyre::Result<()> {
        let backend = self.backend_mut();
        for (address, account_state) in prestate {
            let code = account_state.code.map(Bytecode::new_raw).unwrap_or_default();
            let info = revm::state::AccountInfo {
                nonce: account_state.nonce.unwrap_or_default(),
                balance: account_state.balance.unwrap_or_default(),
                code_hash: keccak256(code.original_byte_slice()),
                code: Some(code),
                account_id: Default::default(),
            };
            backend.insert_account_info(address, info);

            for (slot, value) in account_state.storage {
                let slot = U256::from_be_bytes(slot.0);
                let value = U256::from_be_bytes(value.0);
                backend.insert_account_storage(address, slot, value)?;
            }
        }
        Ok(())
    }

    /// Returns `true` if the account has no code.
    pub fn is_empty_code(&self, address: Address) -> BackendResult<bool> {
        Ok(self.backend().basic_ref(address)?.map(|acc| acc.is_empty_code_hash()).unwrap_or(true))
    }

    #[inline]
    pub fn set_trace_requirements(&mut self, requirements: TraceRequirements) -> &mut Self {
        self.inspector_mut().tracing_requirements(requirements);
        self
    }

    #[inline]
    pub fn set_script_execution(&mut self, script_address: Address) {
        self.inspector_mut().script(script_address);
    }

    #[inline]
    pub fn set_trace_printer(&mut self, trace_printer: bool) -> &mut Self {
        self.inspector_mut().print(trace_printer);
        self
    }

    #[inline]
    pub fn create2_deployer(&self) -> Address {
        self.inspector().create2_deployer
    }

    /// Deploys a contract and commits the new state to the underlying database.
    ///
    /// Executes a CREATE transaction with the contract `code` and persistent database state
    /// modifications.
    pub fn deploy(
        &mut self,
        from: Address,
        code: Bytes,
        value: U256,
        rd: Option<&RevertDecoder>,
    ) -> Result<DeployResult<FEN>, EvmError<FEN>> {
        let (evm_env, tx_env) = self.build_test_env(from, TxKind::Create, code, value);
        self.deploy_with_env(evm_env, tx_env, rd)
    }

    /// Deploys a contract with explicit network-specific context.
    pub fn deploy_with_context(
        &mut self,
        from: Address,
        code: Bytes,
        value: U256,
        chain_context: ChainFor<FEN>,
        rd: Option<&RevertDecoder>,
    ) -> Result<DeployResult<FEN>, EvmError<FEN>> {
        let (evm_env, tx_env) = self.build_test_env(from, TxKind::Create, code, value);
        self.deploy_with_env_and_context(evm_env, tx_env, chain_context, rd)
    }

    /// Deploys a contract using the given `env` and commits the new state to the underlying
    /// database.
    ///
    /// # Panics
    ///
    /// Panics if `tx_env.kind` is not `TxKind::Create(_)`.
    #[instrument(name = "deploy", level = "debug", skip_all)]
    pub fn deploy_with_env(
        &mut self,
        evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
        rd: Option<&RevertDecoder>,
    ) -> Result<DeployResult<FEN>, EvmError<FEN>> {
        let chain_context = self.chain_context_for_synthetic_transaction(&tx_env)?;
        self.deploy_with_env_and_context(evm_env, tx_env, chain_context, rd)
    }

    /// Deploys a contract with explicit network-specific context and commits its state changes.
    ///
    /// # Panics
    ///
    /// Panics if `tx_env.kind` is not `TxKind::Create(_)`.
    #[instrument(name = "deploy", level = "debug", skip_all)]
    pub fn deploy_with_env_and_context(
        &mut self,
        evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
        chain_context: ChainFor<FEN>,
        rd: Option<&RevertDecoder>,
    ) -> Result<DeployResult<FEN>, EvmError<FEN>> {
        assert!(
            matches!(tx_env.kind(), TxKind::Create),
            "Expected create transaction, got {:?}",
            tx_env.kind()
        );
        trace!(sender=%tx_env.caller(), "deploying contract");

        let mut result = self.transact_with_env_and_context(evm_env, tx_env, chain_context)?;
        result = result.into_result(rd)?;
        let Some(Output::Create(_, Some(address))) = result.out else {
            panic!("Deployment succeeded, but no address was returned: {result:#?}");
        };

        // also mark this library as persistent, this will ensure that the state of the library is
        // persistent across fork swaps in forking mode
        self.backend_mut().add_persistent_account(address);

        trace!(%address, "deployed contract");

        Ok(DeployResult { raw: result, address })
    }

    /// Calls the `setUp()` function on a contract.
    ///
    /// This will commit any state changes to the underlying database.
    ///
    /// Ayn changes made during the setup call to env's block environment are persistent, for
    /// example `vm.chainId()` will change the `block.chainId` for all subsequent test calls.
    #[instrument(name = "setup", level = "debug", skip_all)]
    pub fn setup(
        &mut self,
        from: Option<Address>,
        to: Address,
        rd: Option<&RevertDecoder>,
    ) -> Result<RawCallResult<FEN>, EvmError<FEN>> {
        trace!(?from, ?to, "setting up contract");

        let from = from.unwrap_or(CALLER);
        self.backend_mut().set_test_contract(to).set_caller(from);
        let calldata = Bytes::from_static(&ITest::setUpCall::SELECTOR);
        let mut res = self.transact_raw(from, to, calldata, U256::ZERO)?;
        res = res.into_result(rd)?;

        // record any changes made to the block's environment during setup
        self.evm_env_mut().block_env = res.evm_env.block_env.clone();
        // and also the chainid, which can be set manually
        self.evm_env_mut().cfg_env.chain_id = res.evm_env.cfg_env.chain_id;

        let success =
            self.is_raw_call_success(to, Cow::Borrowed(&res.state_changeset), &res, false);
        if !success {
            return Err(res.into_execution_error("execution error".to_string()).into());
        }

        Ok(res)
    }

    /// Performs a call to an account on the current state of the VM.
    pub fn call(
        &self,
        from: Address,
        to: Address,
        func: &Function,
        args: &[DynSolValue],
        value: U256,
        rd: Option<&RevertDecoder>,
    ) -> Result<CallResult<DynSolValue, FEN>, EvmError<FEN>> {
        let calldata = Bytes::from(func.abi_encode_input(args)?);
        let result = self.call_raw(from, to, calldata, value)?;
        result.into_decoded_result(func, rd)
    }

    /// Performs a call to an account on the current state of the VM.
    pub fn call_sol<C: SolCall>(
        &self,
        from: Address,
        to: Address,
        args: &C,
        value: U256,
        rd: Option<&RevertDecoder>,
    ) -> Result<CallResult<C::Return, FEN>, EvmError<FEN>> {
        let calldata = Bytes::from(args.abi_encode());
        let mut raw = self.call_raw(from, to, calldata, value)?;
        raw = raw.into_result(rd)?;
        Ok(CallResult { decoded_result: C::abi_decode_returns(&raw.result)?, raw })
    }

    /// Performs a call to an account on the current state of the VM.
    pub fn transact(
        &mut self,
        from: Address,
        to: Address,
        func: &Function,
        args: &[DynSolValue],
        value: U256,
        rd: Option<&RevertDecoder>,
    ) -> Result<CallResult<DynSolValue, FEN>, EvmError<FEN>> {
        let calldata = Bytes::from(func.abi_encode_input(args)?);
        let result = self.transact_raw(from, to, calldata, value)?;
        result.into_decoded_result(func, rd)
    }

    /// Performs a raw call to an account on the current state of the VM.
    pub fn call_raw(
        &self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let (evm_env, tx_env) = self.build_test_env(from, TxKind::Call(to), calldata, value);
        self.call_with_env(evm_env, tx_env)
    }

    /// Performs a raw call to an account on the current state of the VM with an EIP-7702
    /// authorization list.
    pub fn call_raw_with_authorization(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        authorization_list: Vec<SignedAuthorization>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let (evm_env, mut tx_env) = self.build_test_env(from, to.into(), calldata, value);
        tx_env.set_signed_authorization(authorization_list);
        tx_env.set_tx_type(4);
        self.call_with_env(evm_env, tx_env)
    }

    /// Performs a raw call to an account on the current state of the VM.
    pub fn transact_raw(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let (evm_env, tx_env) = self.build_test_env(from, TxKind::Call(to), calldata, value);
        self.transact_with_env(evm_env, tx_env)
    }

    /// Performs a raw call with explicit network-specific context.
    pub fn transact_raw_with_context(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        chain_context: ChainFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let (evm_env, tx_env) = self.build_test_env(from, TxKind::Call(to), calldata, value);
        self.transact_with_env_and_context(evm_env, tx_env, chain_context)
    }

    /// Performs a raw call to an account on the current state of the VM with an EIP-7702
    /// authorization last.
    pub fn transact_raw_with_authorization(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
        authorization_list: Vec<SignedAuthorization>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let (evm_env, mut tx_env) = self.build_test_env(from, TxKind::Call(to), calldata, value);
        tx_env.set_signed_authorization(authorization_list);
        tx_env.set_tx_type(4);
        self.transact_with_env(evm_env, tx_env)
    }

    /// Applies the EIP-4788 beacon roots system call (Cancun+).
    /// <https://eips.ethereum.org/EIPS/eip-4788>
    pub fn apply_beacon_root(
        &mut self,
        parent_beacon_block_root: alloy_primitives::B256,
    ) -> eyre::Result<()> {
        let calldata = Bytes::copy_from_slice(parent_beacon_block_root.as_slice());
        let mut evm_env = self.evm_env.clone();
        let inspector = self.inspector().clone();
        let mut state = {
            let mut backend = CowBackend::new_borrowed(self.backend());
            let mut evm = FEN::EvmFactory::default().create_foundry_evm_with_inspector(
                &mut backend,
                evm_env.clone(),
                ChainFor::<FEN>::for_transaction(&TxEnvFor::<FEN>::default()),
                inspector,
            );
            let result =
                evm.transact_system_call(SYSTEM_ADDRESS, BEACON_ROOTS_ADDRESS, calldata)?;
            evm_env = evm.finish().1;
            result.state
        };
        state.retain(|address, _| *address == BEACON_ROOTS_ADDRESS);

        self.backend_mut().commit(state);
        self.inspector_mut().set_block(evm_env.block_env);

        Ok(())
    }

    /// Execute the transaction configured in `tx_env`.
    ///
    /// The state after the call is **not** persisted.
    #[instrument(name = "call", level = "debug", skip_all)]
    pub fn call_with_env(
        &self,
        evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let chain_context = self.chain_context_for_synthetic_transaction(&tx_env)?;
        self.call_with_env_and_context(evm_env, tx_env, chain_context)
    }

    /// Executes the transaction with explicit network-specific context without committing state.
    #[instrument(name = "call", level = "debug", skip_all)]
    pub fn call_with_env_and_context(
        &self,
        mut evm_env: EvmEnvFor<FEN>,
        mut tx_env: TxEnvFor<FEN>,
        chain_context: ChainFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let mut stack = self.inspector().clone();
        let sancov_edges = stack.inner.sancov_edges;
        let sancov_trace_cmp = stack.inner.sancov_trace_cmp;
        let sancov_active = sancov_edges || sancov_trace_cmp;
        let mut backend = CowBackend::new_borrowed(self.backend());
        let result = {
            let _guard = sancov_active.then(|| SancovGuard::new(sancov_edges, sancov_trace_cmp));
            backend.inspect_with_context(&mut evm_env, &mut tx_env, chain_context, &mut stack)?
        };
        let has_state_snapshot_failure = backend.has_state_snapshot_failure();
        let fork_block_number = backend.active_fork_block_number();
        let mut result = convert_executed_result(
            evm_env,
            tx_env,
            stack,
            result,
            &backend,
            has_state_snapshot_failure,
            fork_block_number,
        )?;
        if sancov_edges {
            SancovGuard::append_edges_into(&mut result);
        }
        if sancov_trace_cmp {
            SancovGuard::drain_cmp_into(&mut result);
        }
        Ok(result)
    }

    /// Execute the transaction configured in `tx_env`.
    #[instrument(name = "transact", level = "debug", skip_all)]
    pub fn transact_with_env(
        &mut self,
        evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let chain_context = self.chain_context_for_synthetic_transaction(&tx_env)?;
        self.transact_with_env_and_context(evm_env, tx_env, chain_context)
    }

    /// Executes and commits the transaction with explicit network-specific context.
    #[instrument(name = "transact", level = "debug", skip_all)]
    pub fn transact_with_env_and_context(
        &mut self,
        mut evm_env: EvmEnvFor<FEN>,
        mut tx_env: TxEnvFor<FEN>,
        chain_context: ChainFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let mut stack = self.inspector().clone();
        let sancov_edges = stack.inner.sancov_edges;
        let sancov_trace_cmp = stack.inner.sancov_trace_cmp;
        let sancov_active = sancov_edges || sancov_trace_cmp;
        let backend = self.backend_mut();
        let result = {
            let _guard = sancov_active.then(|| SancovGuard::new(sancov_edges, sancov_trace_cmp));
            backend.inspect_with_context(&mut evm_env, &mut tx_env, chain_context, &mut stack)?
        };
        let has_state_snapshot_failure = backend.has_state_snapshot_failure();
        let fork_block_number = backend.active_fork_block_number();
        let mut result = convert_executed_result(
            evm_env,
            tx_env,
            stack,
            result,
            &*backend,
            has_state_snapshot_failure,
            fork_block_number,
        )?;
        if sancov_edges {
            SancovGuard::append_edges_into(&mut result);
        }
        if sancov_trace_cmp {
            SancovGuard::drain_cmp_into(&mut result);
        }
        let committed_tx = result.tx_env.clone();
        self.commit(&mut result);
        self.record_block_transaction(committed_tx);
        Ok(result)
    }

    /// Replays ordinary transactions and executes the target against one EVM instance.
    #[instrument(name = "transact_block_replay", level = "debug", skip_all)]
    pub fn transact_with_ordinary_block_replay(
        &mut self,
        mut evm_env: EvmEnvFor<FEN>,
        target_tx_env: TxEnvFor<FEN>,
        replay: Vec<(B256, TxEnvFor<FEN>)>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let block_number = evm_env.block_env.number();
        let mut stack = self.inspector().clone();
        let sancov_edges = stack.inner.sancov_edges;
        let sancov_trace_cmp = stack.inner.sancov_trace_cmp;
        let sancov_active = sancov_edges || sancov_trace_cmp;
        let backend = self.backend_mut();

        let (result, evm_env, tx_env) = {
            let caller = target_tx_env.caller();
            backend.set_caller(caller).set_spec_id(evm_env.cfg_env.spec);
            let target_contract = match target_tx_env.kind() {
                TxKind::Call(to) => to,
                // The prefix has not run yet, so use the canonical target nonce rather than the
                // current database nonce.
                TxKind::Create => caller.create(target_tx_env.nonce()),
            };
            backend.set_test_contract(target_contract);
            let target_chain_context = ChainFor::<FEN>::for_transaction(&target_tx_env);
            if !replay.is_empty() {
                evm_env.cfg_env.disable_balance_check = true;
            }
            let evm = FEN::EvmFactory::default().create_foundry_evm_with_inspector(
                backend,
                evm_env,
                target_chain_context,
                &mut stack,
            );
            let mut evm = evm;
            evm.disable_inspector();
            for (tx_hash, tx_env) in replay {
                let created = match tx_env.kind() {
                    TxKind::Create => Some(tx_env.caller().create(tx_env.nonce())),
                    TxKind::Call(_) => None,
                };
                let result = evm.transact(tx_env).wrap_err_with(|| {
                    format!("Failed to execute transaction: {tx_hash:?} in block {block_number}")
                })?;
                if result.result.is_success()
                    && let Some(address) = created
                {
                    evm.db_mut().add_persistent_account(address);
                }
                evm.db_mut().commit(result.state);
            }

            evm.enable_inspector();
            let _guard = sancov_active.then(|| SancovGuard::new(sancov_edges, sancov_trace_cmp));
            let result = evm.transact(target_tx_env).wrap_err("EVM error")?;
            let tx_env = evm.tx().clone();
            let evm_env = evm.finish().1;
            (result, evm_env, tx_env)
        };

        let has_state_snapshot_failure = backend.has_state_snapshot_failure();
        let fork_block_number = backend.active_fork_block_number();
        let mut result = convert_executed_result(
            evm_env,
            tx_env,
            stack,
            result,
            &*backend,
            has_state_snapshot_failure,
            fork_block_number,
        )?;
        if sancov_edges {
            SancovGuard::append_edges_into(&mut result);
        }
        if sancov_trace_cmp {
            SancovGuard::drain_cmp_into(&mut result);
        }
        self.commit(&mut result);
        Ok(result)
    }

    /// Tries to execute and commit a canonical system transaction during replay.
    #[cfg(feature = "monad")]
    #[instrument(name = "transact_system_replay", level = "debug", skip_all)]
    pub fn try_transact_system_replay_with_env_and_context(
        &mut self,
        mut evm_env: EvmEnvFor<FEN>,
        mut tx_env: TxEnvFor<FEN>,
        chain_context: ChainFor<FEN>,
    ) -> eyre::Result<Option<RawCallResult<FEN>>> {
        let mut stack = self.inspector().clone();
        let mut backend = CowBackend::new_borrowed(self.backend());
        let Some(result) = backend.try_inspect_system_replay_with_context(
            &mut evm_env,
            &mut tx_env,
            chain_context,
            &mut stack,
        )?
        else {
            return Ok(None);
        };
        let has_state_snapshot_failure = backend.has_state_snapshot_failure();
        let fork_block_number = backend.active_fork_block_number();
        let mut result = convert_executed_result(
            evm_env,
            tx_env,
            stack,
            result,
            &backend,
            has_state_snapshot_failure,
            fork_block_number,
        )?;
        let committed_tx = result.tx_env.clone();
        self.commit(&mut result);
        self.record_block_transaction(committed_tx);
        Ok(Some(result))
    }

    /// Commit the changeset to the database and adjust `self.inspector_config` values according to
    /// the executed call result.
    ///
    /// This should not be exposed to the user, as it should be called only by `transact*`.
    #[instrument(name = "commit", level = "debug", skip_all)]
    fn commit(&mut self, result: &mut RawCallResult<FEN>) {
        // Persist changes to db.
        self.backend_mut().commit(result.state_changeset.clone());

        // Persist cheatcode state.
        self.inspector_mut().cheatcodes = result.cheatcodes.take();
        if let Some(cheats) = self.inspector_mut().cheatcodes.as_mut() {
            // Clear broadcastable transactions
            cheats.broadcastable_transactions.clear();
            cheats.ignored_traces.ignored.clear();
            // if tracing was paused but never unpaused, we should begin next frame with tracing
            // still paused
            if let Some(last_pause_call) = cheats.ignored_traces.last_pause_call.as_mut() {
                *last_pause_call = (0, 0);
            }
        }

        // Persist the changed environment.
        self.inspector_mut().set_block(result.evm_env.block_env.clone());
        self.inspector_mut().set_gas_price(result.tx_env.gas_price());
    }

    /// Returns `true` if a test can be considered successful.
    ///
    /// This is the same as [`Self::is_success`], but will consume the `state_changeset` map to use
    /// internally when calling `failed()`.
    pub fn is_raw_call_mut_success(
        &self,
        address: Address,
        call_result: &mut RawCallResult<FEN>,
        should_fail: bool,
    ) -> bool {
        self.is_raw_call_success(
            address,
            Cow::Owned(std::mem::take(&mut call_result.state_changeset)),
            call_result,
            should_fail,
        )
    }

    /// Returns `true` if a test can be considered successful.
    ///
    /// This is the same as [`Self::is_success`], but intended for outcomes of [`Self::call_raw`].
    pub fn is_raw_call_success(
        &self,
        address: Address,
        state_changeset: Cow<'_, StateChangeset>,
        call_result: &RawCallResult<FEN>,
        should_fail: bool,
    ) -> bool {
        if call_result.has_state_snapshot_failure {
            // a failure occurred in a reverted snapshot, which is considered a failed test
            return should_fail;
        }
        self.is_success(address, call_result.reverted, state_changeset, should_fail)
    }

    /// Like [`Self::is_raw_call_mut_success`] but uses [`Self::is_success_handler_gate`] under
    /// the hood. Intended for invariant view-call success checks during a campaign where the
    /// committed `GLOBAL_FAIL_SLOT` may be stale poison from a previously-recorded handler bug.
    pub fn is_raw_call_mut_success_handler_gate(
        &self,
        address: Address,
        call_result: &mut RawCallResult<FEN>,
    ) -> bool {
        if call_result.has_state_snapshot_failure {
            return false;
        }
        let state_changeset = std::mem::take(&mut call_result.state_changeset);
        self.is_success_handler_gate(address, call_result.reverted, Cow::Owned(state_changeset))
    }

    /// Returns `true` if a test can be considered successful.
    ///
    /// If the call succeeded, we also have to check the global and local failure flags.
    ///
    /// These are set by the test contract itself when an assertion fails, using the internal `fail`
    /// function. The global flag is located in [`CHEATCODE_ADDRESS`] at slot [`GLOBAL_FAIL_SLOT`],
    /// and the local flag is located in the test contract at an unspecified slot.
    ///
    /// This behavior is inherited from Dapptools, where initially only a public
    /// `failed` variable was used to track test failures, and later, a global failure flag was
    /// introduced to track failures across multiple contracts in
    /// [ds-test#30](https://github.com/dapphub/ds-test/pull/30).
    ///
    /// The assumption is that the test runner calls `failed` on the test contract to determine if
    /// it failed. However, we want to avoid this as much as possible, as it is relatively
    /// expensive to set up an EVM call just for checking a single boolean flag.
    ///
    /// See:
    /// - Newer DSTest: <https://github.com/dapphub/ds-test/blob/e282159d5170298eb2455a6c05280ab5a73a4ef0/src/test.sol#L47-L63>
    /// - Older DSTest: <https://github.com/dapphub/ds-test/blob/9ca4ecd48862b40d7b0197b600713f64d337af12/src/test.sol#L38-L49>
    /// - forge-std: <https://github.com/foundry-rs/forge-std/blob/19891e6a0b5474b9ea6827ddb90bb9388f7acfc0/src/StdAssertions.sol#L38-L44>
    pub fn is_success(
        &self,
        address: Address,
        reverted: bool,
        state_changeset: Cow<'_, StateChangeset>,
        should_fail: bool,
    ) -> bool {
        let success = self.is_success_raw(address, reverted, state_changeset, false);
        should_fail ^ success
    }

    /// Like [`Self::is_success`] but ignores the *committed* `GLOBAL_FAIL_SLOT` and only treats
    /// the slot as failed when this call's in-flight changeset writes it. Used by the invariant
    /// runner's per-call handler-success gate, where a `1` already in committed storage is just
    /// stale poison from a previously-recorded handler bug (separately tracked) and must not
    /// suppress later `assert_invariants` / `afterInvariant` evaluations.
    pub fn is_success_handler_gate(
        &self,
        address: Address,
        reverted: bool,
        state_changeset: Cow<'_, StateChangeset>,
    ) -> bool {
        self.is_success_raw(address, reverted, state_changeset, true)
    }

    #[instrument(name = "is_success", level = "debug", skip_all)]
    fn is_success_raw(
        &self,
        address: Address,
        reverted: bool,
        state_changeset: Cow<'_, StateChangeset>,
        pending_global_failure_only: bool,
    ) -> bool {
        // The call reverted.
        if reverted {
            return false;
        }

        // A failure occurred in a reverted snapshot, which is considered a failed test.
        if self.backend().has_state_snapshot_failure() {
            return false;
        }

        // Check the global failure slot. Callers that already track recorded handler bugs
        // out-of-band can pass `pending_global_failure_only = true` to ignore the committed
        // slot (which would otherwise stay `1` for the rest of the run after a non-reverting
        // `vm.assert*` under `assertions_revert = false`).
        let global_failed = if pending_global_failure_only {
            Self::has_pending_global_failure(&state_changeset)
        } else {
            self.has_global_failure(&state_changeset)
        };
        if global_failed {
            return false;
        }

        if !self.legacy_assertions {
            return true;
        }

        // Finally, resort to calling `DSTest::failed`.
        {
            // Construct a new bare-bones backend to evaluate success.
            let mut backend = self.backend().clone_empty();

            // We only clone the test contract and cheatcode accounts,
            // that's all we need to evaluate success.
            for address in [address, CHEATCODE_ADDRESS] {
                let Ok(acc) = self.backend().basic_ref(address) else { return false };
                backend.insert_account_info(address, acc.unwrap_or_default());
            }

            // If this test failed any asserts, then this changeset will contain changes
            // `false -> true` for the contract's `failed` variable and the `globalFailure` flag
            // in the state of the cheatcode address,
            // which are both read when we call `"failed()(bool)"` in the next step.
            backend.commit(state_changeset.into_owned());

            // Check if a DSTest assertion failed
            let executor = self.clone_with_backend(backend);
            let call = executor.call_sol(CALLER, address, &ITest::failedCall {}, U256::ZERO, None);
            match call {
                Ok(CallResult { raw: _, decoded_result: failed }) => {
                    trace!(failed, "DSTest::failed()");
                    !failed
                }
                Err(err) => {
                    trace!(%err, "failed to call DSTest::failed()");
                    true
                }
            }
        }
    }

    /// Returns whether the in-flight state changeset for the current call sets the global
    /// assertion failure flag.
    pub fn has_pending_global_failure(state_changeset: &StateChangeset) -> bool {
        if let Some(acc) = state_changeset.get(&CHEATCODE_ADDRESS)
            && let Some(failed_slot) = acc.storage.get(&GLOBAL_FAIL_SLOT)
            && !failed_slot.present_value().is_zero()
        {
            return true;
        }

        false
    }

    /// Returns whether the global assertion failure flag is set either in the in-flight state
    /// changeset or in the committed backend state.
    pub fn has_global_failure(&self, state_changeset: &StateChangeset) -> bool {
        if Self::has_pending_global_failure(state_changeset) {
            return true;
        }

        self.backend()
            .storage_ref(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)
            .is_ok_and(|failed_slot| !failed_slot.is_zero())
    }

    /// Creates the environment to use when executing a transaction in a test context
    ///
    /// If using a backend with cheatcodes, `tx.gas_price` and `block.number` will be overwritten by
    /// the cheatcode state in between calls.
    fn build_test_env(
        &self,
        caller: Address,
        kind: TxKind,
        data: Bytes,
        value: U256,
    ) -> (EvmEnvFor<FEN>, TxEnvFor<FEN>) {
        let mut cfg_env = self.evm_env.cfg_env.clone();
        cfg_env.spec = self.spec_id();

        // We always set the gas price to 0 so we can execute the transaction regardless of
        // network conditions - the actual gas price is kept in `self.block` and is applied
        // by the cheatcode handler if it is enabled
        let mut block_env = self.evm_env.block_env.clone();
        block_env.set_basefee(0);
        block_env.set_gas_limit(self.gas_limit);

        let mut tx_env = self.tx_env.clone();
        tx_env.set_caller(caller);
        tx_env.set_kind(kind);
        tx_env.set_data(data);
        tx_env.set_value(value);
        // As above, we set the gas price to 0.
        tx_env.set_gas_price(0);
        tx_env.set_gas_priority_fee(None);
        tx_env.set_gas_limit(self.gas_limit);
        tx_env.set_chain_id(Some(self.evm_env.cfg_env.chain_id));

        (EvmEnv { cfg_env, block_env }, tx_env)
    }

    pub fn call_sol_default<C: SolCall>(&self, to: Address, args: &C) -> C::Return
    where
        C::Return: Default,
    {
        self.call_sol(CALLER, to, args, U256::ZERO, None)
            .map(|c| c.decoded_result)
            .inspect_err(|e| warn!(target: "forge::test", "failed calling {:?}: {e}", C::SIGNATURE))
            .unwrap_or_default()
    }
}

/// Represents the context after an execution error occurred.
#[derive(Debug, thiserror::Error)]
#[error("execution reverted: {reason} (gas: {})", raw.gas_used)]
pub struct ExecutionErr<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    /// The raw result of the call.
    pub raw: RawCallResult<FEN>,
    /// The revert reason.
    pub reason: String,
}

impl<FEN: FoundryEvmNetwork> std::ops::Deref for ExecutionErr<FEN> {
    type Target = RawCallResult<FEN>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl<FEN: FoundryEvmNetwork> std::ops::DerefMut for ExecutionErr<FEN> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.raw
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvmError<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    /// Error which occurred during execution of a transaction.
    #[error(transparent)]
    Execution(Box<ExecutionErr<FEN>>),
    /// Error which occurred during ABI encoding/decoding.
    #[error(transparent)]
    Abi(#[from] alloy_dyn_abi::Error),
    /// Error caused which occurred due to calling the `skip` cheatcode.
    #[error("{0}")]
    Skip(SkipReason),
    /// Any other error.
    #[error("{0}")]
    Eyre(
        #[from]
        #[source]
        eyre::Report,
    ),
}

impl<FEN: FoundryEvmNetwork> From<ExecutionErr<FEN>> for EvmError<FEN> {
    fn from(err: ExecutionErr<FEN>) -> Self {
        Self::Execution(Box::new(err))
    }
}

impl<FEN: FoundryEvmNetwork> From<alloy_sol_types::Error> for EvmError<FEN> {
    fn from(err: alloy_sol_types::Error) -> Self {
        Self::Abi(err.into())
    }
}

/// The result of a deployment.
#[derive(Debug)]
pub struct DeployResult<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    /// The raw result of the deployment.
    pub raw: RawCallResult<FEN>,
    /// The address of the deployed contract
    pub address: Address,
}

impl<FEN: FoundryEvmNetwork> std::ops::Deref for DeployResult<FEN> {
    type Target = RawCallResult<FEN>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl<FEN: FoundryEvmNetwork> std::ops::DerefMut for DeployResult<FEN> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.raw
    }
}

impl<FEN: FoundryEvmNetwork> From<DeployResult<FEN>> for RawCallResult<FEN> {
    fn from(d: DeployResult<FEN>) -> Self {
        d.raw
    }
}

/// The result of a raw call.
#[derive(Debug)]
pub struct RawCallResult<FEN: FoundryEvmNetwork = EthEvmNetwork> {
    /// The status of the call
    pub exit_reason: Option<InstructionResult>,
    /// Whether the call was halted by the execution cancellation inspector.
    pub execution_cancelled: bool,
    /// Whether the call reverted or not
    pub reverted: bool,
    /// Whether the call includes a snapshot failure
    ///
    /// This is tracked separately from revert because a snapshot failure can occur without a
    /// revert, since assert failures are stored in a global variable (ds-test legacy)
    pub has_state_snapshot_failure: bool,
    /// The raw result of the call.
    pub result: Bytes,
    /// The gas used for the call
    pub gas_used: u64,
    /// Refunded gas
    pub gas_refunded: u64,
    /// The initial gas stipend for the transaction
    pub stipend: u64,
    /// The logs emitted during the call
    pub logs: Vec<Log>,
    /// The labels assigned to addresses during the call
    pub labels: AddressHashMap<String>,
    /// The traces of the call
    pub traces: Option<SparsedTraceArena>,
    /// Runtime bytecodes for contracts seen in the trace, used by debug source mapping.
    pub debug_bytecodes: AddressHashMap<Bytes>,
    /// The line coverage info collected during the call
    pub line_coverage: Option<HitMaps>,
    /// The edge coverage info collected during the call
    pub edge_coverage: Option<EdgeCoverage>,
    /// EVM comparison operands collected during the call.
    pub evm_cmp_values: Option<Vec<CmpOperands>>,
    /// Observed sub-calls collected during the call.
    pub observed_calls: Vec<ObservedCall>,
    /// Sancov edge coverage from instrumented native Rust crates (e.g. precompiles).
    /// Tracked separately from EVM edge coverage to avoid ID-space collisions.
    pub sancov_coverage: Option<Vec<u8>>,
    /// Comparison operands captured via sancov trace-cmp callbacks.
    pub sancov_cmp_values: Option<Vec<foundry_evm_sancov::CmpSample>>,
    /// Scripted transactions generated from this call
    pub transactions: Option<BroadcastableTransactions<FEN::Network>>,
    /// The changeset of the state.
    pub state_changeset: StateChangeset,
    /// The `EvmEnv` after the call
    pub evm_env: EvmEnvFor<FEN>,
    /// The `TxEnv` after the call
    pub tx_env: TxEnvFor<FEN>,
    /// The cheatcode states after execution
    pub cheatcodes: Option<Box<Cheatcodes<FEN>>>,
    /// The raw output of the execution
    pub out: Option<Output>,
    /// The active fork's block number after execution, if any.
    pub fork_block_number: Option<u64>,
    /// The chisel state
    pub chisel_state: Option<(Vec<U256>, Vec<u8>)>,
    pub reverter: Option<Address>,
    /// Revert payloads minted by the `skip` cheatcode during this call.
    ///
    /// Moved out of the cheatcode state on conversion since `commit` moves that state back into
    /// the executor before results are classified.
    pub skip_payloads: Vec<Bytes>,
}

impl<FEN: FoundryEvmNetwork> Default for RawCallResult<FEN> {
    fn default() -> Self {
        Self {
            exit_reason: None,
            execution_cancelled: false,
            reverted: false,
            has_state_snapshot_failure: false,
            result: Bytes::new(),
            gas_used: 0,
            gas_refunded: 0,
            stipend: 0,
            logs: Vec::new(),
            labels: HashMap::default(),
            traces: None,
            debug_bytecodes: HashMap::default(),
            line_coverage: None,
            edge_coverage: None,
            evm_cmp_values: None,
            observed_calls: Vec::new(),
            sancov_coverage: None,
            sancov_cmp_values: None,
            transactions: None,
            state_changeset: HashMap::default(),
            evm_env: EvmEnv::default(),
            tx_env: TxEnvFor::<FEN>::default(),
            cheatcodes: Default::default(),
            out: None,
            fork_block_number: None,
            chisel_state: None,
            reverter: None,
            skip_payloads: Vec::new(),
        }
    }
}

impl<FEN: FoundryEvmNetwork> RawCallResult<FEN> {
    /// Unpacks an EVM result.
    pub fn from_evm_result(r: Result<Self, EvmError<FEN>>) -> eyre::Result<(Self, Option<String>)> {
        match r {
            Ok(r) => Ok((r, None)),
            Err(EvmError::Execution(e)) => Ok((e.raw, Some(e.reason))),
            Err(e) => Err(e.into()),
        }
    }

    /// Returns the skip reason if this call reverted with a genuine `vm.skip` payload.
    ///
    /// The revert data must byte-equal a payload recorded by the skip cheatcode during this call;
    /// a matching `FOUNDRY::SKIP` prefix alone (user-crafted revert data) does not count.
    pub fn skip_reason(&self) -> Option<SkipReason> {
        if !self.reverted || !self.skip_payloads.contains(&self.result) {
            return None;
        }
        SkipReason::decode(&self.result)
    }

    /// Converts the result of the call into an `EvmError`.
    pub fn into_evm_error(self, rd: Option<&RevertDecoder>) -> EvmError<FEN> {
        if let Some(reason) = self.skip_reason() {
            return EvmError::Skip(reason);
        }
        let reason = rd.unwrap_or_default().decode(&self.result, self.exit_reason);
        EvmError::Execution(Box::new(self.into_execution_error(reason)))
    }

    /// Converts the result of the call into an `ExecutionErr`.
    pub const fn into_execution_error(self, reason: String) -> ExecutionErr<FEN> {
        ExecutionErr { raw: self, reason }
    }

    /// Returns an `EvmError` if the call failed, otherwise returns `self`.
    pub fn into_result(self, rd: Option<&RevertDecoder>) -> Result<Self, EvmError<FEN>> {
        if let Some(reason) = self.exit_reason
            && reason.is_ok()
        {
            Ok(self)
        } else {
            Err(self.into_evm_error(rd))
        }
    }

    /// Decodes the result of the call with the given function.
    pub fn into_decoded_result(
        mut self,
        func: &Function,
        rd: Option<&RevertDecoder>,
    ) -> Result<CallResult<DynSolValue, FEN>, EvmError<FEN>> {
        self = self.into_result(rd)?;
        let mut result = func.abi_decode_output(&self.result)?;
        let decoded_result =
            if result.len() == 1 { result.pop().unwrap() } else { DynSolValue::Tuple(result) };
        Ok(CallResult { raw: self, decoded_result })
    }

    /// Returns the transactions generated from this call.
    pub fn transactions(&self) -> Option<&BroadcastableTransactions<FEN::Network>> {
        self.cheatcodes.as_ref().map(|c| &c.broadcastable_transactions)
    }

    /// Update provided history map with edge coverage info collected during this call.
    pub fn merge_edge_coverage(
        &mut self,
        history_map: &mut Vec<u8>,
        edge_indices: &mut EdgeIndexMap,
    ) -> (bool, bool) {
        let mut new_coverage = false;
        let mut is_edge = false;
        if let Some(x) = &mut self.edge_coverage {
            match x {
                EdgeCoverage::Hash(x) => {
                    if history_map.len() < x.len() {
                        history_map.resize(x.len(), 0);
                    }
                    // Iterate over the current map and the history map together and update
                    // the history map, if we discover some new coverage, report true
                    for (curr, hist) in std::iter::zip(x.iter_mut(), history_map.iter_mut()) {
                        Self::merge_edge_count(*curr, hist, &mut new_coverage, &mut is_edge);

                        // Hash reuses its map; collision-free drains hits.
                        *curr = 0;
                    }
                }
                EdgeCoverage::CollisionFree(hits) => {
                    for hit in hits.drain(..) {
                        let edge_index = edge_indices.edge_index(hit.edge);
                        if history_map.len() <= edge_index {
                            history_map.resize(edge_index + 1, 0);
                        }
                        Self::merge_edge_count(
                            hit.count,
                            &mut history_map[edge_index],
                            &mut new_coverage,
                            &mut is_edge,
                        );
                    }
                }
            }
        }
        (new_coverage, is_edge)
    }

    const fn merge_edge_count(
        curr: u8,
        hist: &mut u8,
        new_coverage: &mut bool,
        is_edge: &mut bool,
    ) {
        let Some(bucket) = Self::bin_count(curr) else {
            return;
        };

        // If the old record for this edge pair is lower, update
        if *hist < bucket {
            if *hist == 0 {
                // Counts as an edge the first time we see it, otherwise it's a feature.
                *is_edge = true;
            }
            *hist = bucket;
            *new_coverage = true;
        }
    }

    /// Convert a hitcount into an AFL-style bucket.
    /// <https://github.com/h0mbre/Lucid/blob/3026e7323c52b30b3cf12563954ac1eaa9c6981e/src/coverage.rs#L57-L85>
    const fn bin_count(count: u8) -> Option<u8> {
        match count {
            0 => None,
            1 => Some(1),
            2 => Some(2),
            3 => Some(4),
            4..=7 => Some(8),
            8..=15 => Some(16),
            16..=31 => Some(32),
            32..=127 => Some(64),
            128..=255 => Some(128),
        }
    }

    /// Update provided history map with sancov coverage info collected during this call.
    /// Uses AFL-style hitcount binning.
    pub fn merge_sancov_coverage(&mut self, history_map: &mut Vec<u8>) -> (bool, bool) {
        let mut new_coverage = false;
        let mut is_edge = false;
        if let Some(x) = &mut self.sancov_coverage {
            if history_map.len() < x.len() {
                history_map.resize(x.len(), 0);
            }
            for (curr, hist) in std::iter::zip(x.iter_mut(), history_map.iter_mut()) {
                if *curr > 0 {
                    if let Some(bucket) = Self::bin_count(*curr)
                        && *hist < bucket
                    {
                        if *hist == 0 {
                            is_edge = true;
                        }
                        *hist = bucket;
                        new_coverage = true;
                    }
                    *curr = 0;
                }
            }
        }
        (new_coverage, is_edge)
    }

    /// Merge both EVM and sancov coverage into their respective history maps.
    /// Returns `(new_coverage, is_edge)` — true if either domain produced new coverage.
    pub fn merge_all_coverage(
        &mut self,
        evm_history: &mut Vec<u8>,
        evm_edge_indices: &mut EdgeIndexMap,
        sancov_history: &mut Vec<u8>,
    ) -> (bool, bool) {
        let (new_evm, edge_evm) = self.merge_edge_coverage(evm_history, evm_edge_indices);
        let (new_san, edge_san) = self.merge_sancov_coverage(sancov_history);
        (new_evm || new_san, edge_evm || edge_san)
    }
}

/// The result of a call.
pub struct CallResult<T = DynSolValue, FEN: FoundryEvmNetwork = EthEvmNetwork> {
    /// The raw result of the call.
    pub raw: RawCallResult<FEN>,
    /// The decoded result of the call.
    pub decoded_result: T,
}

impl<T, FEN: FoundryEvmNetwork> std::ops::Deref for CallResult<T, FEN> {
    type Target = RawCallResult<FEN>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl<T, FEN: FoundryEvmNetwork> std::ops::DerefMut for CallResult<T, FEN> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.raw
    }
}

fn calculate_stipend(tx_env: &impl Transaction, spec: SpecId, eip2780_enabled: bool) -> u64 {
    let eip2780 = eip2780_enabled.then(|| Eip2780TxInfo {
        value: tx_env.value(),
        is_self_transfer: matches!(tx_env.kind(), TxKind::Call(to) if to == tx_env.caller()),
    });
    revm::interpreter::gas::calculate_initial_tx_gas_for_tx(tx_env, spec, eip2780)
        .initial_total_gas()
}

/// Converts the data aggregated in the `inspector` and `call` to a `RawCallResult`.
fn convert_executed_result<FEN: FoundryEvmNetwork, H: IntoInstructionResult>(
    evm_env: EvmEnvFor<FEN>,
    tx_env: TxEnvFor<FEN>,
    mut inspector: InspectorStack<FEN>,
    ResultAndState { result, state: state_changeset }: ResultAndState<H>,
    db: &dyn DatabaseRef<Error = DatabaseError>,
    has_state_snapshot_failure: bool,
    fork_block_number: Option<u64>,
) -> eyre::Result<RawCallResult<FEN>> {
    let execution_cancelled = inspector.execution_cancelled();
    let (exit_reason, gas_refunded, gas_used, out, exec_logs) = match result {
        ExecutionResult::Success { reason, gas, output, logs } => {
            (reason.into(), gas.final_refunded(), gas.tx_gas_used(), Some(output), logs)
        }
        ExecutionResult::Revert { gas, output, logs } => {
            (InstructionResult::Revert, 0_u64, gas.tx_gas_used(), Some(Output::Call(output)), logs)
        }
        ExecutionResult::Halt { reason, gas, logs } => {
            (reason.into_instruction_result(), 0_u64, gas.tx_gas_used(), None, logs)
        }
    };
    let stipend = calculate_stipend(
        &tx_env,
        evm_env.cfg_env.spec.into(),
        evm_env.cfg_env.is_amsterdam_eip2780_enabled(),
    );

    let result = match &out {
        Some(Output::Call(data)) => data.clone(),
        _ => Bytes::new(),
    };
    let observed_calls = inspector
        .inner
        .fuzzer
        .as_mut()
        .map(|fuzzer| fuzzer.take_observed_calls())
        .unwrap_or_default();

    let InspectorData {
        mut logs,
        labels,
        traces,
        line_coverage,
        edge_coverage,
        evm_cmp_values,
        mut cheatcodes,
        chisel_state,
        reverter,
    } = inspector.collect();
    let fork_block_number = cheatcodes
        .as_ref()
        .and_then(|cheats| cheats.fork_block_number_override)
        .or(fork_block_number);
    let debug_bytecodes = collect_debug_bytecodes(traces.as_ref(), db);

    if logs.is_empty() {
        logs = exec_logs;
    }

    let transactions = cheatcodes
        .as_ref()
        .map(|c| c.broadcastable_transactions.clone())
        .filter(|txs| !txs.is_empty());
    let skip_payloads =
        cheatcodes.as_mut().map(|c| std::mem::take(&mut c.skip_payloads)).unwrap_or_default();

    Ok(RawCallResult {
        exit_reason: Some(exit_reason),
        execution_cancelled,
        reverted: !matches!(exit_reason, return_ok!()),
        has_state_snapshot_failure,
        result,
        gas_used,
        gas_refunded,
        stipend,
        logs,
        labels,
        traces,
        debug_bytecodes,
        line_coverage,
        edge_coverage,
        evm_cmp_values,
        observed_calls,
        sancov_coverage: None,
        sancov_cmp_values: None,
        transactions,
        state_changeset,
        evm_env,
        tx_env,
        cheatcodes,
        out,
        fork_block_number,
        chisel_state,
        reverter,
        skip_payloads,
    })
}

fn collect_debug_bytecodes(
    traces: Option<&SparsedTraceArena>,
    db: &dyn DatabaseRef<Error = DatabaseError>,
) -> AddressHashMap<Bytes> {
    let mut bytecodes = HashMap::default();
    let Some(traces) = traces else { return bytecodes };

    for node in traces.arena.nodes() {
        let address = node.trace.address;
        if bytecodes.contains_key(&address) {
            continue;
        }

        let Ok(Some(account)) = db.basic_ref(address) else { continue };
        let code: Option<Bytecode> =
            account.code.or_else(|| db.code_by_hash_ref(account.code_hash).ok());
        let code: Bytes = code.map(|code| code.original_bytes()).unwrap_or_default();

        if !code.is_empty() {
            bytecodes.insert(address, code);
        }
    }

    bytecodes
}

/// Timer for a fuzz test.
pub struct FuzzTestTimer {
    /// Inner fuzz test timer - (test start time, test duration).
    inner: Option<(Instant, Duration)>,
}

impl FuzzTestTimer {
    pub fn new(timeout: Option<u32>) -> Self {
        Self { inner: timeout.map(|timeout| (Instant::now(), Duration::from_secs(timeout.into()))) }
    }

    /// Whether the fuzz test timer is enabled.
    pub const fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// Whether the current fuzz test timed out and should be stopped.
    pub fn is_timed_out(&self) -> bool {
        self.inner.is_some_and(|(start, duration)| start.elapsed() > duration)
    }
}

/// Helper struct to enable early exit behavior: when one test fails or run is interrupted,
/// all other tests stop early.
#[derive(Clone, Debug)]
pub struct EarlyExit {
    /// Shared atomic flag set to `true` when a failure occurs or ctrl-c received.
    inner: Arc<AtomicBool>,
    /// Whether to exit early on test failure (fail-fast mode).
    fail_fast: bool,
}

impl EarlyExit {
    pub fn new(fail_fast: bool) -> Self {
        Self { inner: Arc::new(AtomicBool::new(false)), fail_fast }
    }

    /// Records a test failure. Only triggers early exit if fail-fast mode is enabled.
    pub fn record_failure(&self) {
        if self.fail_fast {
            self.inner.store(true, Ordering::Relaxed);
        }
    }

    /// Records a Ctrl-C interrupt. Always triggers early exit.
    pub fn record_ctrl_c(&self) {
        self.inner.store(true, Ordering::Relaxed);
    }

    /// Whether tests should stop and exit early.
    pub fn should_stop(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }
}

/// Shared cancellation state for an active EVM execution.
#[derive(Clone, Debug)]
pub(crate) enum EvmExecutionCancellation {
    /// Cancellation driven only by the process-wide early-exit signal.
    EarlyExit(EarlyExit),
    /// Cancellation driven by the complete invariant campaign stop condition.
    Campaign { early_exit: EarlyExit, stop: Arc<AtomicBool>, deadline: Option<Instant> },
}

impl EvmExecutionCancellation {
    pub(crate) const fn early_exit(early_exit: EarlyExit) -> Self {
        Self::EarlyExit(early_exit)
    }

    pub(crate) const fn campaign(
        early_exit: EarlyExit,
        stop: Arc<AtomicBool>,
        deadline: Option<Instant>,
    ) -> Self {
        Self::Campaign { early_exit, stop, deadline }
    }

    /// Returns whether execution should stop, optionally polling a campaign deadline.
    pub(crate) fn should_stop(&self, poll_deadline: bool) -> bool {
        match self {
            Self::EarlyExit(early_exit) => early_exit.should_stop(),
            Self::Campaign { early_exit, stop, deadline } => {
                if early_exit.should_stop() || stop.load(Ordering::Relaxed) {
                    return true;
                }
                if poll_deadline && deadline.is_some_and(|deadline| Instant::now() > deadline) {
                    stop.store(true, Ordering::Relaxed);
                    return true;
                }
                false
            }
        }
    }

    pub(crate) fn request_stop(&self) {
        if let Self::Campaign { stop, .. } = self {
            stop.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) const fn early_exit_ref(&self) -> &EarlyExit {
        match self {
            Self::EarlyExit(early_exit) | Self::Campaign { early_exit, .. } => early_exit,
        }
    }
}

/// Returns whether a nested revert can be ignored when fail-on-revert is disabled.
#[inline]
pub fn should_ignore_revert(
    fail_on_revert: bool,
    target: Address,
    reverter: Option<Address>,
    extra_cheatcode_addresses: &[Address],
) -> bool {
    !fail_on_revert
        && reverter.is_some_and(|reverter| {
            reverter != target
                && reverter != CHEATCODE_ADDRESS
                && !extra_cheatcode_addresses.contains(&reverter)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspectors::{EdgeCovHit, EdgeKey};
    use foundry_cheatcodes::{
        CheatsConfig,
        Vm::{blobhashesCall, mockCallRevert_1Call, revertToStateCall, snapshotStateCall},
    };
    use foundry_config::Config;
    #[cfg(feature = "monad")]
    use foundry_evm_core::constants::MONAD_CHEATCODE_ADDRESS;
    use foundry_evm_core::{constants::MAGIC_SKIP, evm::TempoEvmNetwork, opts::EvmOpts};
    use foundry_evm_traces::InternalTraceMode;
    use revm::context::TxEnv;
    use std::{sync::mpsc, thread};

    fn dense_call(edge: EdgeKey) -> RawCallResult {
        RawCallResult {
            edge_coverage: Some(EdgeCoverage::CollisionFree(vec![EdgeCovHit { edge, count: 1 }])),
            ..Default::default()
        }
    }

    #[test]
    fn nested_revert_is_ignored_only_when_allowed() {
        let target = Address::from([0x11; 20]);
        let nested = Address::from([0x22; 20]);

        assert!(should_ignore_revert(false, target, Some(nested), &[]));
        assert!(!should_ignore_revert(true, target, Some(nested), &[]));
        assert!(!should_ignore_revert(false, target, Some(target), &[]));
        assert!(!should_ignore_revert(false, target, Some(CHEATCODE_ADDRESS), &[]));
        assert!(!should_ignore_revert(false, target, None, &[]));
    }

    #[cfg(feature = "monad")]
    #[test]
    fn network_cheatcode_revert_handling_is_monad_specific() {
        let target = Address::from([0x11; 20]);

        assert!(should_ignore_revert(false, target, Some(MONAD_CHEATCODE_ADDRESS), &[]));
        assert!(!should_ignore_revert(
            false,
            target,
            Some(MONAD_CHEATCODE_ADDRESS),
            &[MONAD_CHEATCODE_ADDRESS],
        ));
    }

    #[cfg(feature = "monad")]
    #[test]
    fn executor_tooling_follows_concrete_builder() {
        let ethereum = ExecutorBuilder::<EthEvmNetwork>::new().build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            Backend::spawn(None).unwrap(),
            NetworkConfigs::with_monad(),
        );
        assert!(ethereum.backend().networks().is_monad());
        assert!(!ethereum.backend().is_persistent(&MONAD_CHEATCODE_ADDRESS));

        let monad = ExecutorBuilder::<MonadEvmNetwork>::new().build(
            EvmEnvFor::<MonadEvmNetwork>::default(),
            TxEnvFor::<MonadEvmNetwork>::default(),
            Backend::spawn(None).unwrap(),
            NetworkConfigs::with_monad(),
        );
        assert!(monad.inspector().networks.is_monad());
        assert!(monad.backend().networks().is_monad());
        assert!(monad.backend().is_persistent(&MONAD_CHEATCODE_ADDRESS));
    }

    #[test]
    fn tempo_labels_follow_concrete_builder() {
        let ethereum = ExecutorBuilder::<EthEvmNetwork>::new().build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            Backend::spawn(None).unwrap(),
            NetworkConfigs::with_tempo(),
        );
        assert!(ethereum.inspector().tempo_labels.is_none());

        let tempo = ExecutorBuilder::<TempoEvmNetwork>::new().build(
            EvmEnvFor::<TempoEvmNetwork>::default(),
            TxEnvFor::<TempoEvmNetwork>::default(),
            Backend::spawn(None).unwrap(),
            NetworkConfigs::default(),
        );
        assert!(tempo.inspector().tempo_labels.is_some());
    }

    #[test]
    fn collision_free_edge_merge_uses_stable_indices() {
        let first =
            EdgeKey { address: Address::ZERO, depth: None, pc: 0, jump_dest: U256::from(10) };
        let second =
            EdgeKey { address: Address::ZERO, depth: None, pc: 0, jump_dest: U256::from(20) };
        let mut history = Vec::new();
        let mut edge_indices = EdgeIndexMap::default();

        assert_eq!(
            dense_call(first).merge_edge_coverage(&mut history, &mut edge_indices),
            (true, true)
        );
        assert_eq!(history, [1]);

        assert_eq!(
            dense_call(second).merge_edge_coverage(&mut history, &mut edge_indices),
            (true, true)
        );
        assert_eq!(history, [1, 1]);

        assert_eq!(
            dense_call(first).merge_edge_coverage(&mut history, &mut edge_indices),
            (false, false)
        );
        assert_eq!(history, [1, 1]);
    }

    #[test]
    fn collision_free_edge_merge_handles_sparse_observation_indices() {
        let first =
            EdgeKey { address: Address::ZERO, depth: None, pc: 0, jump_dest: U256::from(10) };
        let second =
            EdgeKey { address: Address::ZERO, depth: None, pc: 0, jump_dest: U256::from(20) };
        let mut edge_indices = EdgeIndexMap::default();
        edge_indices.edge_index(first);
        edge_indices.edge_index(second);
        let mut history = Vec::new();

        assert_eq!(
            dense_call(second).merge_edge_coverage(&mut history, &mut edge_indices),
            (true, true)
        );
        assert_eq!(history, [0, 1]);
    }

    #[test]
    fn cheatcode_skip_payload_is_classified_as_skip() {
        let raw = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            result: Bytes::from_static(b"FOUNDRY::SKIPwith reason"),
            skip_payloads: vec![Bytes::from_static(b"FOUNDRY::SKIPwith reason")],
            ..Default::default()
        };

        let err = raw.into_evm_error(None);
        assert!(matches!(err, EvmError::Skip(_)));
    }

    #[test]
    fn forged_skip_payload_is_execution_error() {
        let raw = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            result: Bytes::from_static(MAGIC_SKIP),
            reverter: Some(CHEATCODE_ADDRESS),
            ..Default::default()
        };

        let err = raw.into_evm_error(None);
        assert!(matches!(err, EvmError::Execution(_)));
    }

    #[test]
    fn block_replay_commits_prefix_and_traces_only_target() {
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().gas_limit(1 << 20).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );
        executor.set_balance(CALLER, U256::MAX).unwrap();
        executor.set_trace_requirements(TraceRequirements::none().with_calls(true));

        let address = Address::repeat_byte(0x11);
        // Increment slot zero and return its new value.
        executor
            .set_code(
                address,
                Bytecode::new_raw(Bytes::from_static(&[
                    0x60, 0x00, 0x54, 0x60, 0x01, 0x01, 0x80, 0x60, 0x00, 0x55, 0x60, 0x00, 0x52,
                    0x60, 0x20, 0x60, 0x00, 0xf3,
                ])),
            )
            .unwrap();
        let prefix = TxEnv {
            caller: CALLER,
            gas_limit: 100_000,
            kind: TxKind::Call(address),
            ..Default::default()
        };
        let reverted_create = TxEnv {
            nonce: 1,
            kind: TxKind::Create,
            data: Bytes::from_static(&[0x5f, 0x5f, 0xfd]),
            ..prefix.clone()
        };
        let target = TxEnv { nonce: 2, ..prefix.clone() };

        let result = executor
            .transact_with_ordinary_block_replay(
                EvmEnv::default(),
                target,
                vec![(B256::repeat_byte(1), prefix), (B256::repeat_byte(2), reverted_create)],
            )
            .unwrap();

        assert_eq!(result.result, Bytes::from(U256::from(2).to_be_bytes::<32>()));
        assert_eq!(result.tx_env.nonce, 2);
        assert_eq!(executor.get_nonce(CALLER).unwrap(), 3);
        assert_eq!(executor.backend().storage_ref(address, U256::ZERO).unwrap(), U256::from(2));
        assert_eq!(result.traces.unwrap().arena.nodes().len(), 1);
    }

    #[test]
    fn block_replay_initializes_create_target_from_canonical_nonce() {
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().gas_limit(1 << 20).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );
        executor.set_balance(CALLER, U256::MAX).unwrap();

        let prefix = TxEnv {
            caller: CALLER,
            gas_limit: 100_000,
            kind: TxKind::Call(Address::repeat_byte(0x11)),
            ..Default::default()
        };
        let target = TxEnv {
            nonce: 1,
            kind: TxKind::Create,
            data: Bytes::from_static(&[0x00]),
            ..prefix.clone()
        };
        let expected = CALLER.create(1);

        let result = executor
            .transact_with_ordinary_block_replay(
                EvmEnv::default(),
                target,
                vec![(B256::repeat_byte(1), prefix)],
            )
            .unwrap();

        assert!(
            matches!(result.out, Some(Output::Create(_, Some(address))) if address == expected)
        );
        assert!(executor.backend().is_persistent(&expected));
        assert!(executor.backend().has_cheatcode_access(&expected));
    }

    #[test]
    fn block_replay_preserves_successful_prefix_deployment() {
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().gas_limit(1 << 20).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );
        executor.set_balance(CALLER, U256::MAX).unwrap();

        let deployed = CALLER.create(0);
        let prefix = TxEnv {
            caller: CALLER,
            gas_limit: 100_000,
            kind: TxKind::Create,
            data: Bytes::from_static(&[0x00]),
            ..Default::default()
        };
        let target = TxEnv { nonce: 1, kind: TxKind::Call(deployed), ..prefix.clone() };

        executor
            .transact_with_ordinary_block_replay(
                EvmEnv::default(),
                target,
                vec![(B256::repeat_byte(1), prefix)],
            )
            .unwrap();

        assert!(executor.backend().is_persistent(&deployed));
    }

    #[cfg(feature = "monad")]
    #[test]
    fn block_replay_executes_monad_system_prefix() {
        use foundry_evm_core::evm::MonadEvmNetwork;

        let backend = Backend::<MonadEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::<MonadEvmNetwork>::default().gas_limit(1 << 20).build(
            EvmEnvFor::<MonadEvmNetwork>::default(),
            TxEnvFor::<MonadEvmNetwork>::default(),
            backend,
            NetworkConfigs::with_monad(),
        );
        executor.set_balance(CALLER, U256::MAX).unwrap();

        let system_address = alloy_primitives::address!("6f49a8f621353f12378d0046e7d7e4b9b249dc9e");
        let staking_address =
            alloy_primitives::address!("0000000000000000000000000000000000001000");
        let selector = keccak256("syscallSnapshot()");
        let system = TxEnv {
            caller: system_address,
            gas_limit: 0,
            kind: TxKind::Call(staking_address),
            data: Bytes::copy_from_slice(&selector[..4]),
            chain_id: None,
            ..Default::default()
        };
        let target = TxEnv {
            caller: CALLER,
            gas_limit: 100_000,
            kind: TxKind::Call(Address::repeat_byte(0x11)),
            ..Default::default()
        };
        let system_chain = ChainFor::<MonadEvmNetwork>::for_transaction(&system);
        let target_chain = ChainFor::<MonadEvmNetwork>::for_transaction(&target);

        let (result, used_system_replay) = executor
            .transact_with_monad_block_replay(
                EvmEnvFor::<MonadEvmNetwork>::default(),
                target,
                target_chain,
                vec![(B256::repeat_byte(1), system, system_chain)],
                false,
            )
            .unwrap()
            .unwrap();

        assert!(!used_system_replay);
        assert!(!result.reverted);
        assert_eq!(executor.get_nonce(system_address).unwrap(), 1);
    }

    #[cfg(feature = "monad")]
    #[test]
    fn block_replay_executes_monad_system_target() {
        use foundry_evm_core::evm::MonadEvmNetwork;

        let backend = Backend::<MonadEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::<MonadEvmNetwork>::default().gas_limit(1 << 20).build(
            EvmEnvFor::<MonadEvmNetwork>::default(),
            TxEnvFor::<MonadEvmNetwork>::default(),
            backend,
            NetworkConfigs::with_monad(),
        );

        let system_address = alloy_primitives::address!("6f49a8f621353f12378d0046e7d7e4b9b249dc9e");
        let staking_address =
            alloy_primitives::address!("0000000000000000000000000000000000001000");
        let selector = keccak256("syscallSnapshot()");
        let target = TxEnv {
            caller: system_address,
            gas_limit: 0,
            kind: TxKind::Call(staking_address),
            data: Bytes::copy_from_slice(&selector[..4]),
            chain_id: None,
            ..Default::default()
        };
        let target_chain = ChainFor::<MonadEvmNetwork>::for_transaction(&target);

        let (result, used_system_replay) = executor
            .transact_with_monad_block_replay(
                EvmEnvFor::<MonadEvmNetwork>::default(),
                target,
                target_chain,
                Vec::new(),
                false,
            )
            .unwrap()
            .unwrap();

        assert!(used_system_replay);
        assert!(!result.reverted);
        assert_eq!(executor.get_nonce(system_address).unwrap(), 1);
    }

    #[test]
    fn mismatched_skip_payload_is_execution_error() {
        let raw = RawCallResult::<EthEvmNetwork> {
            reverted: true,
            result: Bytes::from_static(b"FOUNDRY::SKIPforged"),
            skip_payloads: vec![Bytes::from_static(b"FOUNDRY::SKIPgenuine")],
            ..Default::default()
        };

        let err = raw.into_evm_error(None);
        assert!(matches!(err, EvmError::Execution(_)));
    }

    #[test]
    fn set_spec_id_updates_spec_dependent_cfg_state() {
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );

        executor.evm_env_mut().cfg_env.set_spec_and_mainnet_gas_params(SpecId::HOMESTEAD);
        assert_eq!(
            executor.evm_env().cfg_env.gas_params(),
            &revm::context_interface::cfg::GasParams::new_spec(SpecId::HOMESTEAD),
        );
        assert!(!executor.evm_env().cfg_env.is_amsterdam_eip8037_enabled());

        executor.set_spec_id(SpecId::AMSTERDAM);

        assert_eq!(executor.spec_id(), SpecId::AMSTERDAM);
        assert_eq!(
            executor.evm_env().cfg_env.gas_params(),
            &revm::context_interface::cfg::GasParams::new_spec(SpecId::AMSTERDAM),
        );
        assert!(executor.evm_env().cfg_env.is_amsterdam_eip8037_enabled());
    }

    #[test]
    fn calculate_stipend_uses_eip2780_transaction_context() {
        let caller = Address::repeat_byte(0x11);
        let recipient = Address::repeat_byte(0x22);
        let mut tx = TxEnv { caller, kind: TxKind::Call(recipient), ..Default::default() };

        assert_eq!(
            calculate_stipend(&tx, SpecId::AMSTERDAM, true),
            revm::primitives::eip2780::TX_BASE_COST
                + revm::primitives::eip8038::COLD_ACCOUNT_ACCESS
        );
        assert_eq!(
            calculate_stipend(&tx, SpecId::AMSTERDAM, false),
            revm::context_interface::cfg::GasParams::new_spec(SpecId::AMSTERDAM).tx_base_stipend()
        );

        tx.kind = TxKind::Call(caller);
        assert_eq!(
            calculate_stipend(&tx, SpecId::AMSTERDAM, true),
            revm::primitives::eip2780::TX_BASE_COST
        );
    }

    #[test]
    fn amsterdam_intercepted_create_refunds_state_gas() {
        let cheats_config =
            Arc::new(CheatsConfig::new(&Config::default(), EvmOpts::default(), None, None, false));
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default()
            .inspectors(|stack| stack.cheatcodes(cheats_config))
            .spec_id(SpecId::AMSTERDAM)
            .gas_limit(1_000_000)
            .build(EvmEnv::default(), TxEnv::default(), backend, NetworkConfigs::default());

        let target = Address::repeat_byte(0x11);
        // PUSH0; PUSH0; PUSH0; CREATE; POP; STOP.
        executor
            .set_code(
                target,
                Bytecode::new_raw(Bytes::from_static(&[0x5f, 0x5f, 0x5f, 0xf0, 0x50, 0x00])),
            )
            .unwrap();
        executor.inspector_mut().cheatcodes.as_mut().unwrap().intercept_next_create_call = true;

        let result = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();

        assert!(!result.reverted);
        assert!(
            result.gas_used
                < revm::context_interface::cfg::GasParams::new_spec(SpecId::AMSTERDAM)
                    .create_state_gas(),
            "failed CREATE retained its conditional state-gas charge"
        );
    }

    #[test]
    fn amsterdam_mocked_call_revert_refunds_state_gas() {
        let cheats_config =
            Arc::new(CheatsConfig::new(&Config::default(), EvmOpts::default(), None, None, false));
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default()
            .inspectors(|stack| stack.cheatcodes(cheats_config))
            .spec_id(SpecId::AMSTERDAM)
            .gas_limit(1_000_000)
            .build(EvmEnv::default(), TxEnv::default(), backend, NetworkConfigs::default());

        let target = Address::repeat_byte(0x11);
        let mocked = Address::repeat_byte(0x22);
        executor
            .transact_raw(
                CALLER,
                CHEATCODE_ADDRESS,
                mockCallRevert_1Call {
                    callee: mocked,
                    msgValue: U256::from(1),
                    data: Bytes::new(),
                    revertData: Bytes::new(),
                }
                .abi_encode()
                .into(),
                U256::ZERO,
            )
            .unwrap();
        executor.set_code(mocked, Bytecode::default()).unwrap();
        executor.set_balance(target, U256::from(1)).unwrap();

        // PUSH0 x4; PUSH1 1; PUSH20 <mocked>; GAS; CALL; POP; STOP.
        let mut code = vec![0x5f, 0x5f, 0x5f, 0x5f, 0x60, 0x01, 0x73];
        code.extend_from_slice(mocked.as_slice());
        code.extend_from_slice(&[0x5a, 0xf1, 0x50, 0x00]);
        executor.set_code(target, Bytecode::new_raw(code.into())).unwrap();

        let result = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();

        assert!(!result.reverted);
        assert!(
            result.gas_used
                < revm::context_interface::cfg::GasParams::new_spec(SpecId::AMSTERDAM)
                    .new_account_state_gas(),
            "reverted mocked CALL retained its conditional state-gas charge"
        );
    }

    #[test]
    fn set_trace_requirements_replaces_trace_mode_between_transactions() {
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().gas_limit(1 << 20).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );
        executor.evm_env_mut().cfg_env.disable_nonce_check = true;
        let target = Address::repeat_byte(0x11);
        // PUSH1 4; JUMP; STOP; JUMPDEST; PUSH1 1; PUSH1 0; SSTORE; STOP.
        executor
            .set_code(
                target,
                Bytecode::new_raw(Bytes::from_static(&[
                    0x60, 0x04, 0x56, 0x00, 0x5b, 0x60, 0x01, 0x60, 0x00, 0x55, 0x00,
                ])),
            )
            .unwrap();

        let untraced = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        assert!(untraced.traces.is_none());

        executor.set_trace_requirements(TraceRequirements::none().with_debug(true));
        let debug = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        let debug_steps = &debug.traces.as_ref().unwrap().nodes()[0].trace.steps;
        assert_eq!(debug_steps.len(), 7);
        assert!(debug_steps.iter().all(|step| step.stack.is_some() && step.memory.is_some()));

        executor.set_trace_requirements(
            TraceRequirements::none().with_decode_internal(InternalTraceMode::Full),
        );
        let internal = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        let internal_steps = &internal.traces.as_ref().unwrap().nodes()[0].trace.steps;
        assert_eq!(internal_steps.len(), 2);
        assert!(internal_steps.iter().all(|step| step.stack.is_some() && step.memory.is_some()));

        executor.set_trace_requirements(TraceRequirements::none());
        let untraced = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        assert!(untraced.traces.is_none());
    }

    #[test]
    fn early_exit_interrupts_active_evm_execution() {
        const GAS_LIMIT: u64 = 1 << 24;
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().gas_limit(GAS_LIMIT).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );
        let early_exit = EarlyExit::new(false);
        executor.inspector_mut().set_early_exit(early_exit.clone());

        let target = Address::repeat_byte(0x11);
        // JUMPDEST; PUSH1 0; JUMP loops until the inspector observes the interrupt.
        executor
            .set_code(target, Bytecode::new_raw(Bytes::from_static(&[0x5b, 0x60, 0x00, 0x56])))
            .unwrap();

        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO);
            let _ = result_tx.send(result);
        });

        started_rx.recv().unwrap();
        thread::sleep(Duration::from_millis(1));
        early_exit.record_ctrl_c();

        let result = result_rx.recv_timeout(Duration::from_secs(1));
        handle.join().unwrap();
        let result = result.expect("active EVM execution did not observe early exit").unwrap();
        assert!(result.execution_cancelled);
        assert!(!result.reverted);
        assert_eq!(result.exit_reason, Some(InstructionResult::Stop));
        assert!(result.gas_used > 21_000, "interrupt fired before EVM execution started");
        assert!(result.gas_used < GAS_LIMIT, "execution ran out of gas instead of exiting");
    }

    #[test]
    fn completed_execution_is_not_retroactively_cancelled() {
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().gas_limit(1 << 24).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );
        let early_exit = EarlyExit::new(false);
        executor.inspector_mut().set_early_exit(early_exit.clone());

        let target = Address::repeat_byte(0x11);
        executor.set_code(target, Bytecode::new_raw(Bytes::from_static(&[0x00]))).unwrap();
        let result = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        early_exit.record_ctrl_c();

        assert!(!result.execution_cancelled);
        assert!(!result.reverted);
    }

    #[test]
    fn campaign_deadline_interrupts_active_evm_execution() {
        const GAS_LIMIT: u64 = 1 << 24;
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().gas_limit(GAS_LIMIT).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );
        let cancellation = EvmExecutionCancellation::campaign(
            EarlyExit::new(false),
            Arc::new(AtomicBool::new(false)),
            Some(Instant::now()),
        );
        executor.inspector_mut().set_execution_cancellation(cancellation);

        let target = Address::repeat_byte(0x11);
        executor
            .set_code(target, Bytecode::new_raw(Bytes::from_static(&[0x5b, 0x60, 0x00, 0x56])))
            .unwrap();

        let result = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        assert!(result.execution_cancelled);
        assert!(!result.reverted);
        assert_eq!(result.exit_reason, Some(InstructionResult::Stop));
        assert!(result.gas_used < GAS_LIMIT, "execution ran out of gas instead of timing out");
    }

    #[test]
    fn beacon_root_system_call_does_not_persist_system_address() {
        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default().spec_id(SpecId::CANCUN).build(
            EvmEnvFor::<EthEvmNetwork>::default(),
            TxEnvFor::<EthEvmNetwork>::default(),
            backend,
            NetworkConfigs::default(),
        );
        let before = executor.backend().basic_ref(SYSTEM_ADDRESS).unwrap();

        executor.apply_beacon_root(B256::repeat_byte(0x11)).unwrap();

        assert_eq!(
            executor.backend().basic_ref(SYSTEM_ADDRESS).unwrap(),
            before,
            "EIP-4788 system calls must not persist the system caller account",
        );
    }

    /// Regression test for `pre_override_blob_hashes` restoration.
    ///
    /// Exercises the `None` arm of `sync_tx_after_env_override_restore` with
    /// *non-empty* native blob hashes, the case that cannot be reached from
    /// Solidity because no cheatcode sets `tx.blob_hashes` without also setting
    /// `env_overrides.blob_hashes`.
    ///
    /// Steps:
    /// 1. Seed `tx.blob_hashes = original` directly (no cheatcode -> override stays `None`).
    /// 2. `vm.snapshotState()` -> `inner_snapshot_state` captures `pre_override_blob_hashes =
    ///    Some(original)`.
    /// 3. `vm.blobhashes(new)` -> sets override (`Some`) AND real tx hashes.
    /// 4. `vm.revertToState(id)` -> restores override to `None`,
    ///    `sync_tx_after_env_override_restore` must restore `tx.blob_hashes = original`.
    #[test]
    fn pre_override_blob_hashes_restored_on_revert_to_state() {
        let cheats_config =
            Arc::new(CheatsConfig::new(&Config::default(), EvmOpts::default(), None, None, false));

        let backend = Backend::<EthEvmNetwork>::spawn(None).unwrap();
        let mut executor = ExecutorBuilder::default()
            .inspectors(|stack| stack.cheatcodes(cheats_config))
            .spec_id(SpecId::CANCUN)
            .build(EvmEnv::default(), TxEnv::default(), backend, NetworkConfigs::default());

        let original: Vec<B256> = vec![B256::repeat_byte(0x11), B256::repeat_byte(0x22)];
        executor.tx_env_mut().set_blob_hashes(original.clone());

        let snap_result = executor
            .transact_raw(
                CALLER,
                CHEATCODE_ADDRESS,
                snapshotStateCall {}.abi_encode().into(),
                U256::ZERO,
            )
            .expect("snapshotState failed");
        assert!(!snap_result.reverted, "snapshotState reverted unexpectedly");
        let snapshot_id = U256::from_be_slice(&snap_result.result[..32]);

        let new_hashes = vec![B256::repeat_byte(0x33)];
        let blob_result = executor
            .transact_raw(
                CALLER,
                CHEATCODE_ADDRESS,
                blobhashesCall { hashes: new_hashes }.abi_encode().into(),
                U256::ZERO,
            )
            .expect("blobhashes failed");
        assert!(!blob_result.reverted, "blobhashes reverted unexpectedly");

        let revert_result = executor
            .transact_raw(
                CALLER,
                CHEATCODE_ADDRESS,
                revertToStateCall { snapshotId: snapshot_id }.abi_encode().into(),
                U256::ZERO,
            )
            .expect("revertToState failed");
        assert!(!revert_result.reverted, "revertToState reverted unexpectedly");

        assert_eq!(
            revert_result.tx_env.blob_hashes, original,
            "pre_override_blob_hashes must be restored to original non-empty hashes, not []",
        );
        assert!(
            executor.inspector().cheatcodes.as_ref().unwrap().env_overrides.is_empty(),
            "inactive env overrides must be removed after restoring their metadata",
        );
    }
}
