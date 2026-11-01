//! EVM executor abstractions, which can execute calls.
//!
//! Used for running tests, scripts, and interacting with the inner backend which holds the state.

use crate::inspectors::{
    Cheatcodes, CmpOperands, EdgeCoverage, EdgeIndexMap, InspectorData, InspectorStack,
    cheatcodes::BroadcastableTransactions,
};
use alloy_consensus::transaction::Recovered;
use alloy_dyn_abi::{DynSolValue, FunctionExt, JsonAbiExt};
use alloy_eips::{Typed2718, eip4788::BEACON_ROOTS_ADDRESS, eip7702::SignedAuthorization};
use alloy_json_abi::Function;
use alloy_network::TransactionBuilder;
use alloy_primitives::{
    Address, B256, Bytes, Log, TxKind, U256, keccak256,
    map::{AddressHashMap, HashMap},
};
use alloy_sol_types::{SolCall, sol};
use evm2::{
    Evm, TxResultWithState,
    bytecode::Bytecode,
    evm::{AccountInfo, PendingState, SystemTx},
    interpreter::InstrStop,
};
use foundry_cheatcodes::CheatcodesBackend;
use foundry_common::FoundryTransactionBuilder;
use foundry_evm_core::{
    backend::{Backend, BackendResult, CowBackend, GLOBAL_FAIL_SLOT, handler_error},
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
        ChainFor, EthEvmNetwork, EvmEnv, EvmEnvFor, EvmFactoryFor, FoundryEvmExt,
        FoundryEvmFactory, FoundryEvmFor, FoundryEvmNetwork, SpecFor, TransactionRequestFor, TxFor,
        TxResultFor, evm_env, release_inspector,
    },
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_fuzz::ObservedCall;
use foundry_evm_networks::NetworkConfigs;
use foundry_evm_traces::{SparsedTraceArena, TraceRequirements};
use sancov::SancovGuard;
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(feature = "monad")]
use foundry_evm_core::evm::{BlockContext, MonadEvmNetwork};

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
pub use trace::{TracingExecutor, TracingFork};

const DURATION_BETWEEN_METRICS_REPORT: Duration = Duration::from_secs(5);

/// EIP-8130 transaction type.
const EIP8130_TX_TYPE: u8 = 0x79;

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
/// The executor can be configured with various evm2 inspectors, like `Cheatcodes`.
///
/// There are multiple ways of interacting the EVM:
/// - `call`: executes a transaction, but does not persist any state changes; similar to `eth_call`,
///   where the EVM state is unchanged after the call.
/// - `transact`: executes a transaction and persists the state changes
/// - `deploy`: a special case of `transact`, specialized for persisting the state of a contract
///   deployment
/// - `setup`: a special case of `transact`, used to set up the environment for a test
///
/// Every execution builds an EVM from the backend, installs the inspector stack, which holds the
/// backend for the run, and then parks the EVM into the backend to commit or drops it to discard.
#[derive(Clone, Debug)]
pub struct Executor<FEN: FoundryEvmNetwork> {
    /// The backend holding the accepted state.
    ///
    /// Wrapped in `Arc` for efficient cloning during parallel fuzzing. Use [`Arc::make_mut`]
    /// for copy-on-write semantics when mutation is needed.
    backend: Arc<Backend<FEN>>,
    /// The EVM environment (spec, runtime version and block).
    evm_env: EvmEnvFor<FEN>,
    /// The transaction defaults of the executor's calls, such as the sender and gas price.
    tx: TransactionRequestFor<FEN>,
    /// The inspector stack.
    inspector: InspectorStack<FEN>,
    /// The gas limit for calls and deployments.
    gas_limit: u64,
    /// Whether `failed()` should be called on the test contract to determine if the test failed.
    legacy_assertions: bool,
}

impl<FEN: FoundryEvmNetwork> Executor<FEN> {
    /// Creates a new `Executor` with the given arguments.
    #[inline]
    pub fn new(
        mut backend: Backend<FEN>,
        evm_env: EvmEnvFor<FEN>,
        tx: TransactionRequestFor<FEN>,
        inspector: InspectorStack<FEN>,
        networks: NetworkConfigs,
        gas_limit: u64,
        legacy_assertions: bool,
    ) -> Self {
        backend.set_networks(networks);
        let extra_cheatcode_addresses = inspector.extra_cheatcode_addresses();
        backend.extend_persistent_accounts(extra_cheatcode_addresses.iter().copied());

        // Need to create a non-empty contract on the cheatcodes address so `extcodesize` checks
        // do not fail.
        // The code hash value does not matter, as long as it's not zero or `KECCAK_EMPTY`.
        let stub = Bytecode::new_raw(Bytes::from_static(&[0]));
        backend.insert_account_info(
            CHEATCODE_ADDRESS,
            AccountInfo::new(U256::ZERO, 0, CHEATCODE_CONTRACT_HASH, Some(stub.clone())),
        );
        for &address in extra_cheatcode_addresses {
            backend.insert_account_info(
                address,
                AccountInfo::new(U256::ZERO, 0, keccak256(address), Some(stub.clone())),
            );
        }

        // Foundry enables `EIP7702` on every spec, so the EIP-2935 predeploy keys off the spec.
        if !backend.is_forked_mode() && evm_env.spec.into() >= evm2::SpecId::PRAGUE {
            let account = backend.basic(HISTORY_STORAGE_ADDRESS).ok().flatten().unwrap_or_default();
            backend.insert_account_info(
                HISTORY_STORAGE_ADDRESS,
                account.with_code(Bytecode::new_raw(HISTORY_STORAGE_CODE.clone())),
            );

            let current_block = evm_env.block.number;
            let mut block_number = history_window_start(current_block);
            while block_number < current_block {
                let block_hash = backend.block_hash(block_number).unwrap_or_default();
                let slot = history_storage_slot(block_number);
                let value = history_storage_value(block_hash);
                backend.insert_account_storage(HISTORY_STORAGE_ADDRESS, slot, value);
                block_number += U256::ONE;
            }
        }

        Self { backend: Arc::new(backend), evm_env, tx, inspector, gas_limit, legacy_assertions }
    }

    fn clone_with_backend(&self, backend: Backend<FEN>) -> Self {
        Self {
            backend: Arc::new(backend),
            evm_env: self.evm_env,
            tx: self.tx.clone(),
            inspector: self.inspector().clone(),
            gas_limit: self.gas_limit,
            legacy_assertions: self.legacy_assertions,
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

    /// Returns a reference to the EVM environment.
    pub const fn evm_env(&self) -> &EvmEnvFor<FEN> {
        &self.evm_env
    }

    /// Returns a mutable reference to the EVM environment.
    pub const fn evm_env_mut(&mut self) -> &mut EvmEnvFor<FEN> {
        &mut self.evm_env
    }

    /// Returns the transaction defaults of the executor's calls.
    pub const fn tx(&self) -> &TransactionRequestFor<FEN> {
        &self.tx
    }

    /// Returns the transaction defaults of the executor's calls mutably.
    pub const fn tx_mut(&mut self) -> &mut TransactionRequestFor<FEN> {
        &mut self.tx
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
        self.evm_env.spec
    }

    /// Sets the EVM spec and updates spec-dependent gas parameters.
    pub fn set_spec_id(&mut self, spec_id: SpecFor<FEN>) {
        self.evm_env = EvmFactoryFor::<FEN>::default().env_with_spec(self.evm_env, spec_id);
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
        // evm2's local database reports unknown accounts as absent.
        let create2_deployer_account = self.account(DEFAULT_CREATE2_DEPLOYER)?;

        // If the deployer is not currently deployed, deploy the default one.
        if create2_deployer_account.code.is_none_or(|code| code.is_empty()) {
            let creator = DEFAULT_CREATE2_DEPLOYER_DEPLOYER;

            // Probably 0, but just in case.
            let initial_balance = self.get_balance(creator)?;
            self.set_balance(creator, U256::MAX)?;

            // The deployer is test harness state, so the debugger does not step through it.
            let recorder = self.inspector_mut().debug_recorder.take();
            let res = self.deploy(creator, DEFAULT_CREATE2_DEPLOYER_CODE.into(), U256::ZERO, None);
            self.inspector_mut().debug_recorder = recorder;
            let res = res?;
            trace!(create2=?res.address, "deployed local create2 deployer");

            self.set_balance(creator, initial_balance)?;
        }
        Ok(())
    }

    /// Returns the accepted account at `address`, or an empty account.
    fn account(&self, address: Address) -> BackendResult<AccountInfo> {
        Ok(self.backend().basic(address)?.unwrap_or_default())
    }

    /// Set the balance of an account.
    pub fn set_balance(&mut self, address: Address, amount: U256) -> BackendResult<()> {
        trace!(?address, ?amount, "setting account balance");
        let account = self.account(address)?.with_balance(amount);
        self.backend_mut().insert_account_info(address, account);
        Ok(())
    }

    /// Gets the balance of an account
    pub fn get_balance(&self, address: Address) -> BackendResult<U256> {
        Ok(self.account(address)?.balance)
    }

    /// Sets the nonce of an account without modifying the transaction defaults.
    pub fn set_account_nonce(&mut self, address: Address, nonce: u64) -> BackendResult<()> {
        let account = self.account(address)?.with_nonce(nonce);
        self.backend_mut().insert_account_info(address, account);
        Ok(())
    }

    /// Sets the nonce of an account and the transaction defaults.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) -> BackendResult<()> {
        self.set_account_nonce(address, nonce)?;
        self.tx_mut().set_nonce(nonce);
        Ok(())
    }

    /// Returns the nonce of an account.
    pub fn get_nonce(&self, address: Address) -> BackendResult<u64> {
        Ok(self.account(address)?.nonce)
    }

    /// Set the code of an account.
    pub fn set_code(&mut self, address: Address, code: Bytecode) -> BackendResult<()> {
        let account = self.account(address)?.with_code(code);
        self.backend_mut().insert_account_info(address, account);
        Ok(())
    }

    /// Set the storage of an account.
    pub fn set_storage(
        &mut self,
        address: Address,
        storage: HashMap<U256, U256>,
    ) -> BackendResult<()> {
        self.backend_mut().replace_account_storage(address, storage);
        Ok(())
    }

    /// Set a storage slot of an account.
    pub fn set_storage_slot(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> BackendResult<()> {
        self.backend_mut().insert_account_storage(address, slot, value);
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
            let info = AccountInfo::default()
                .with_balance(account_state.balance.unwrap_or_default())
                .with_nonce(account_state.nonce.unwrap_or_default())
                .with_code(code);
            backend.insert_account_info(address, info);

            for (slot, value) in account_state.storage {
                backend.insert_account_storage(address, slot.into(), value.into());
            }
        }
        Ok(())
    }

    /// Returns `true` if the account has no code.
    pub fn is_empty_code(&self, address: Address) -> BackendResult<bool> {
        Ok(self
            .backend()
            .basic(address)?
            .is_none_or(|acc| acc.code_hash == alloy_primitives::KECCAK256_EMPTY))
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
        let (evm_env, tx) = self.prepare_call_env(from, TxKind::Create, code, value);
        self.deploy_with_env(evm_env, tx, rd)
    }

    /// Deploys a contract using the given environment and transaction and commits the new state to
    /// the underlying database.
    ///
    /// # Panics
    ///
    /// Panics if `tx` is not a create transaction.
    #[instrument(name = "deploy", level = "debug", skip_all)]
    pub fn deploy_with_env(
        &mut self,
        evm_env: EvmEnvFor<FEN>,
        tx: TransactionRequestFor<FEN>,
        rd: Option<&RevertDecoder>,
    ) -> Result<DeployResult<FEN>, EvmError<FEN>> {
        assert!(
            tx.kind().is_some_and(|kind| kind.is_create()),
            "Expected create transaction, got {:?}",
            tx.kind()
        );
        trace!(sender=?tx.from(), "deploying contract");

        let result = self.transact_with_env(evm_env, tx)?;
        self.persist_deployment(result, rd)
    }

    /// Turns the committed result of a creation into a deployment, marking the created contract
    /// persistent so that its state survives fork switches.
    ///
    /// # Panics
    ///
    /// Panics if a successful creation returned no address.
    pub fn persist_deployment(
        &mut self,
        result: RawCallResult<FEN>,
        rd: Option<&RevertDecoder>,
    ) -> Result<DeployResult<FEN>, EvmError<FEN>> {
        let result = result.into_result(rd)?;
        let Some(address) = result.created_address else {
            panic!("Deployment succeeded, but no address was returned: {result:#?}");
        };
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
        self.evm_env_mut().block = res.evm_env.block;
        // and also the spec and chain ID, which can be set manually
        if res.evm_env.spec != self.spec_id() {
            self.set_spec_id(res.evm_env.spec);
        }
        self.evm_env_mut().set_chain_id(res.evm_env.chain_id());

        let success = self.is_raw_call_success(to, Cow::Borrowed(&res.state_changeset), &res);
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
        let (evm_env, tx) = self.prepare_call_env(from, TxKind::Call(to), calldata, value);
        self.call_with_env(evm_env, tx)
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
        let (evm_env, mut tx) = self.prepare_call_env(from, to.into(), calldata, value);
        with_authorization_list(&mut tx, authorization_list);
        self.call_with_env(evm_env, tx)
    }

    /// Performs a raw call to an account on the current state of the VM.
    pub fn transact_raw(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let (evm_env, tx) = self.prepare_call_env(from, TxKind::Call(to), calldata, value);
        self.transact_with_env(evm_env, tx)
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
        let (evm_env, mut tx) = self.prepare_call_env(from, TxKind::Call(to), calldata, value);
        with_authorization_list(&mut tx, authorization_list);
        self.transact_with_env(evm_env, tx)
    }

    /// Applies the EIP-4788 beacon roots system call (Cancun+).
    /// <https://eips.ethereum.org/EIPS/eip-4788>
    pub fn apply_beacon_root(&mut self, parent_beacon_block_root: B256) -> eyre::Result<()> {
        let mut evm = self.backend().create_evm();
        install_env::<FEN>(self.backend(), &mut evm, self.evm_env);
        let pending = evm
            .system_call(SystemTx::new(BEACON_ROOTS_ADDRESS, parent_beacon_block_root.into()))?
            .detach()
            .pending_state;
        drop(evm);

        // Only the beacon roots contract keeps the changes, not the system caller.
        let mut state = PendingState::default();
        for change in pending.account_changes().filter(|c| c.address == BEACON_ROOTS_ADDRESS) {
            state.insert_account(change.address, change.original.cloned(), change.current.cloned());
            for (key, slot) in change.storage.changed_slots() {
                state.insert_storage(change.address, *key, slot.original, slot.current);
            }
        }
        self.backend_mut().commit_pending(&state);
        Ok(())
    }

    /// Executes the transaction `tx` with `evm_env`.
    ///
    /// The state after the call is **not** persisted.
    #[instrument(name = "call", level = "debug", skip_all)]
    pub fn call_with_env(
        &self,
        evm_env: EvmEnvFor<FEN>,
        tx: TransactionRequestFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let init = tx.from().zip(tx.kind());
        let tx_env = FEN::tx_from_request(tx.clone())?;
        let mut result = self.call_tx_with_init(evm_env, &tx_env, None, init)?;
        result.set_stipend(&evm_env, tx);
        Ok(result)
    }

    /// Executes the converted transaction `tx` with `evm_env`, and `ext` or extension state
    /// prepared for `tx`, without persisting its state.
    ///
    /// The result's `stipend` is zero, since `tx` is opaque; see [`calculate_stipend`].
    #[instrument(name = "call", level = "debug", skip_all)]
    pub fn call_tx(
        &self,
        evm_env: EvmEnvFor<FEN>,
        tx: &Recovered<TxFor<FEN>>,
        ext: Option<ChainFor<FEN>>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        self.call_tx_with_init(evm_env, tx, ext, None)
    }

    /// Like [`Self::call_tx`], recording `init`'s caller and test contract, as on master, if the
    /// call modifies the backend.
    fn call_tx_with_init(
        &self,
        evm_env: EvmEnvFor<FEN>,
        tx: &Recovered<TxFor<FEN>>,
        ext: Option<ChainFor<FEN>>,
        init: Option<(Address, TxKind)>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let mut stack = self.inspector().clone();
        let mut backend = CowBackend::new_borrowed(self.backend());
        if let Some((caller, kind)) = init {
            backend.initialize(caller, kind);
        }
        let (execution, evm) = execute(&mut backend, evm_env, tx, ext, &mut stack)?;
        // Retain lifecycle changes; defer pending writes until commit.
        backend.park_detached(evm);
        let mut result = convert_executed_result(execution, stack, &backend)?;
        if let Cow::Owned(backend) = backend.backend {
            result.post_execution_backend = Some(Box::new(backend));
        }
        Ok(result)
    }

    /// Executes the transaction `tx` with `evm_env` and persists its state.
    #[instrument(name = "transact", level = "debug", skip_all)]
    pub fn transact_with_env(
        &mut self,
        evm_env: EvmEnvFor<FEN>,
        tx: TransactionRequestFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        // As on master, every transaction records its caller and test contract.
        if let (Some(caller), Some(kind)) = (tx.from(), tx.kind()) {
            self.backend_mut().initialize(caller, kind);
        }
        let mut result = self.transact_tx(evm_env, &FEN::tx_from_request(tx.clone())?, None)?;
        result.set_stipend(&evm_env, tx);
        Ok(result)
    }

    /// Executes the converted transaction `tx` with `evm_env`, and `ext` or extension state
    /// prepared for `tx`, and persists its state.
    #[instrument(name = "transact", level = "debug", skip_all)]
    pub fn transact_tx(
        &mut self,
        evm_env: EvmEnvFor<FEN>,
        tx: &Recovered<TxFor<FEN>>,
        ext: Option<ChainFor<FEN>>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let mut stack = self.inspector().clone();
        let backend = self.backend_mut();
        let (execution, mut evm) = execute(backend, evm_env, tx, ext, &mut stack)?;
        evm.commit_source(&execution.state);
        backend.park(evm);
        let mut result = convert_executed_result(execution, stack, &*backend)?;
        // The execution already committed its state into the backend.
        self.persist_inspector_state(&mut result);
        Ok(result)
    }

    /// Replays the transactions preceding `target` in its block, then executes `target` with the
    /// inspectors and persists the state of all of them.
    ///
    /// The caller selects which prefix transactions replay. `target_kind` is the target's kind,
    /// which decides the test contract.
    #[instrument(name = "transact_block_replay", level = "debug", skip_all)]
    pub fn transact_with_block_replay(
        &mut self,
        mut evm_env: EvmEnvFor<FEN>,
        target: &Recovered<TxFor<FEN>>,
        target_kind: TxKind,
        replay: &[(B256, Recovered<TxFor<FEN>>)],
    ) -> eyre::Result<RawCallResult<FEN>> {
        self.replay_block_prefix(&mut evm_env, target, target_kind, replay, |evm, tx| {
            evm.ext_mut().for_next_transaction(tx);
        })?;
        self.transact_tx(evm_env, target, None)
    }

    /// Commits the transactions preceding `target` in its block, each with the network extension
    /// state `prepare` sets up for it, and initializes the target's caller and test contract.
    fn replay_block_prefix(
        &mut self,
        evm_env: &mut EvmEnvFor<FEN>,
        target: &Recovered<TxFor<FEN>>,
        target_kind: TxKind,
        replay: &[(B256, Recovered<TxFor<FEN>>)],
        mut prepare: impl FnMut(&mut FoundryEvmFor<'_, FEN>, &Recovered<TxFor<FEN>>),
    ) -> eyre::Result<()> {
        // EIP-8130 requires phase-aware execution and has no ordinary call/create kind.
        // Check the entire prefix before executing anything.
        eyre::ensure!(
            target.ty() != EIP8130_TX_TYPE,
            "EIP-8130 target transactions are not supported by tooling replay"
        );
        for (hash, tx) in replay {
            eyre::ensure!(
                tx.ty() != EIP8130_TX_TYPE,
                "EIP-8130 prefix transaction {hash} is not supported by tooling replay"
            );
        }
        let block_number = evm_env.block.number;
        let caller = target.signer();
        self.backend_mut().set_caller(caller).set_spec_id(evm_env.spec);
        if !replay.is_empty() {
            evm_env.set_feature(evm2::EvmFeatures::BALANCE_CHECK, false);
        }

        for (tx_hash, tx) in replay {
            let backend = self.backend_mut();
            let mut evm = backend.create_evm();
            install_env::<FEN>(backend, &mut evm, *evm_env);
            prepare(&mut evm, tx);
            let result = evm.transact(tx).map(|executed| executed.commit()).map_err(|err| {
                eyre::eyre!(
                    "Failed to execute transaction: {tx_hash:?} in block {block_number}: {err:?}"
                )
            })?;
            backend.park(evm);
            if result.status
                && let Some(address) = result.created_address
            {
                backend.add_persistent_account(address);
            }
        }

        // The prefix has run, so the caller's nonce is the target's canonical nonce.
        self.backend_mut().initialize(caller, target_kind);
        Ok(())
    }

    /// Commits a call's state and inspector. Detached backends replace the executor's backend,
    /// so commit before unrelated backend mutations.
    #[instrument(name = "commit", level = "debug", skip_all)]
    fn commit(&mut self, result: &mut RawCallResult<FEN>) {
        if let Some(mut backend) = result.post_execution_backend.take() {
            // Snapshot failures are call-local.
            backend.set_state_snapshot_failure(self.backend().has_state_snapshot_failure());
            self.backend = Arc::new(*backend);
        }
        self.backend_mut().commit_pending(&result.state_changeset);
        self.persist_inspector_state(result);
    }

    /// Persists the cheatcode state and block of an executed call into the inspector stack.
    fn persist_inspector_state(&mut self, result: &mut RawCallResult<FEN>) {
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
        self.inspector_mut().set_block(result.evm_env.block);
    }

    /// Returns `true` if a test can be considered successful.
    ///
    /// This is the same as [`Self::is_success`], but will consume the `state_changeset` map to use
    /// internally when calling `failed()`.
    pub fn is_raw_call_mut_success(
        &self,
        address: Address,
        call_result: &mut RawCallResult<FEN>,
    ) -> bool {
        self.is_raw_call_success(
            address,
            Cow::Owned(std::mem::take(&mut call_result.state_changeset)),
            call_result,
        )
    }

    /// Returns `true` if a test can be considered successful.
    ///
    /// This is the same as [`Self::is_success`], but intended for outcomes of [`Self::call_raw`].
    pub fn is_raw_call_success(
        &self,
        address: Address,
        state_changeset: Cow<'_, PendingState>,
        call_result: &RawCallResult<FEN>,
    ) -> bool {
        if call_result.has_state_snapshot_failure {
            // a failure occurred in a reverted snapshot, which is considered a failed test
            return false;
        }
        self.is_success_raw(
            address,
            call_result.reverted,
            state_changeset,
            false,
            call_result.post_execution_backend.as_deref().unwrap_or_else(|| self.backend()),
        )
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
        self.is_success_raw(
            address,
            call_result.reverted,
            Cow::Owned(state_changeset),
            true,
            call_result.post_execution_backend.as_deref().unwrap_or_else(|| self.backend()),
        )
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
        state_changeset: Cow<'_, PendingState>,
    ) -> bool {
        self.is_success_raw(address, reverted, state_changeset, false, self.backend())
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
        state_changeset: Cow<'_, PendingState>,
    ) -> bool {
        self.is_success_raw(address, reverted, state_changeset, true, self.backend())
    }

    #[instrument(name = "is_success", level = "debug", skip_all)]
    fn is_success_raw(
        &self,
        address: Address,
        reverted: bool,
        state_changeset: Cow<'_, PendingState>,
        pending_global_failure_only: bool,
        source: &Backend<FEN>,
    ) -> bool {
        // The call reverted.
        if reverted {
            return false;
        }

        // A failure occurred in a reverted snapshot, which is considered a failed test.
        if source.has_state_snapshot_failure() {
            return false;
        }

        // Check the global failure slot. Callers that already track recorded handler bugs
        // out-of-band can pass `pending_global_failure_only = true` to ignore the committed
        // slot (which would otherwise stay `1` for the rest of the run after a non-reverting
        // `vm.assert*` under `assertions_revert = false`).
        let global_failed = Self::has_pending_global_failure(&state_changeset)
            || (!pending_global_failure_only
                && source
                    .storage(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)
                    .is_ok_and(|slot| !slot.is_zero()));
        if global_failed {
            return false;
        }

        if !self.legacy_assertions {
            return true;
        }

        // Finally, resort to calling `DSTest::failed`.
        {
            // Construct a new bare-bones backend to evaluate success.
            let mut backend = source.clone_empty();

            // We only clone the test contract and cheatcode accounts,
            // that's all we need to evaluate success.
            for address in [address, CHEATCODE_ADDRESS] {
                let Ok(acc) = source.basic(address) else { return false };
                backend.insert_account_info(address, acc.unwrap_or_default());
            }

            // If this test failed any asserts, then this changeset will contain changes
            // `false -> true` for the contract's `failed` variable and the `globalFailure` flag
            // in the state of the cheatcode address,
            // which are both read when we call `"failed()(bool)"` in the next step.
            backend.commit_pending(&state_changeset);

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
    pub fn has_pending_global_failure(state_changeset: &PendingState) -> bool {
        state_changeset.account_changes().any(|change| {
            change.address == CHEATCODE_ADDRESS
                && change
                    .storage
                    .slots
                    .get(&GLOBAL_FAIL_SLOT)
                    .is_some_and(|slot| !slot.value.current.is_zero())
        })
    }

    /// Returns whether the global assertion failure flag is set either in the in-flight state
    /// changeset or in the committed backend state.
    pub fn has_global_failure(&self, state_changeset: &PendingState) -> bool {
        if Self::has_pending_global_failure(state_changeset) {
            return true;
        }

        self.backend()
            .storage(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)
            .is_ok_and(|failed_slot| !failed_slot.is_zero())
    }

    /// Creates the environment and transaction to use when executing a call in a test context.
    ///
    /// The transaction executes with no fees and the executor's gas limit, from `caller`'s current
    /// nonce. If using a backend with cheatcodes, the gas price and block are overwritten by the
    /// cheatcode state in between calls.
    pub fn prepare_call_env(
        &self,
        caller: Address,
        kind: TxKind,
        data: Bytes,
        value: U256,
    ) -> (EvmEnvFor<FEN>, TransactionRequestFor<FEN>) {
        // We always set the gas price to 0 so we can execute the transaction regardless of
        // network conditions - the actual gas price is kept in the cheatcode state and is applied
        // by the cheatcode handler if it is enabled.
        let mut evm_env = self.evm_env;
        evm_env.block.basefee = U256::ZERO;
        evm_env.block.gas_limit = U256::from(self.gas_limit);

        let mut tx = self.tx.clone();
        tx.set_from(caller);
        tx.set_kind(kind);
        tx.set_input(data);
        tx.set_value(value);
        tx.set_gas_limit(self.gas_limit);
        tx.set_chain_id(evm_env.chain_id());
        tx.set_nonce(self.get_nonce(caller).unwrap_or_default());
        if tx.max_fee_per_gas().is_some() {
            tx.set_max_fee_per_gas(0);
            tx.set_max_priority_fee_per_gas(0);
        } else {
            tx.set_gas_price(0);
        }

        (evm_env, tx)
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

/// Attaches an EIP-7702 authorization list to `tx`, which then executes as a fee-less EIP-7702
/// transaction.
fn with_authorization_list<B: FoundryTransactionBuilder<N> + TransactionBuilder, N>(
    tx: &mut B,
    authorization_list: Vec<SignedAuthorization>,
) where
    N: alloy_network::Network<TransactionRequest = B>,
{
    tx.set_authorization_list(authorization_list);
    tx.set_max_fee_per_gas(0);
    tx.set_max_priority_fee_per_gas(0);
}

/// Installs `env` on `evm`, freshly created over `backend`, which starts with the backend's
/// environment.
fn install_env<FEN: FoundryEvmNetwork>(
    backend: &Backend<FEN>,
    evm: &mut FoundryEvmFor<'_, FEN>,
    env: EvmEnvFor<FEN>,
) {
    if same_execution_config::<FEN>(&evm_env(evm), &env) {
        evm.set_block(env.block);
        backend.lifecycle().sync_database_policy(evm);
        return;
    }
    // The backend's factory carries network configuration, such as Base's activation admin.
    let lifecycle = backend.lifecycle();
    let factory = lifecycle.factory().configure_for(&lifecycle.forks().active_db(), &env);
    let precompiles = factory.precompiles(&env, evm.ext());
    let EvmEnv { spec, version, block } = env;
    evm.set_block_and_execution_config(
        block,
        factory.execution_config(spec, version),
        spec,
        factory.tx_registry(spec),
        precompiles,
    );
    backend.lifecycle().sync_database_policy(evm);
}

/// Returns whether `current` and `next` build the same execution config: the same spec and every
/// field of their runtime [`Version`](evm2::Version), so only the block differs.
fn same_execution_config<FEN: FoundryEvmNetwork>(
    current: &EvmEnvFor<FEN>,
    next: &EvmEnvFor<FEN>,
) -> bool {
    // `Version` has no `PartialEq`; destructure it so a new field fails to compile here.
    let evm2::Version {
        gas_params,
        features,
        chain_id,
        tx_gas_limit_cap,
        memory_limit,
        max_code_size,
        max_initcode_size,
        max_blobs_per_tx,
        initial_call_depth,
        _non_exhaustive: (),
    } = next.version;
    let version = &current.version;
    current.spec == next.spec
        && version.gas_params == gas_params
        && version.features == features
        && version.chain_id == chain_id
        && version.tx_gas_limit_cap == tx_gas_limit_cap
        && version.memory_limit == memory_limit
        && version.max_code_size == max_code_size
        && version.max_initcode_size == max_initcode_size
        && version.max_blobs_per_tx == max_blobs_per_tx
        && version.initial_call_depth == initial_call_depth
}

#[cfg(feature = "monad")]
impl Executor<MonadEvmNetwork> {
    /// Like [`Executor::transact_with_block_replay`], positioning the prefix and target in the
    /// Monad block `context`. The block is trusted history: its canonical protocol transactions
    /// replay as protocol system calls, and a non-canonical envelope from the protocol sender is
    /// rejected.
    #[instrument(name = "transact_monad_block_replay", level = "debug", skip_all)]
    pub fn transact_with_monad_block_replay(
        &mut self,
        mut env: EvmEnvFor<MonadEvmNetwork>,
        target: &Recovered<TxFor<MonadEvmNetwork>>,
        target_kind: TxKind,
        replay: &[(B256, Recovered<TxFor<MonadEvmNetwork>>)],
        mut context: BlockContext<MonadEvmNetwork>,
    ) -> eyre::Result<RawCallResult<MonadEvmNetwork>> {
        let factory = EvmFactoryFor::<MonadEvmNetwork>::default();
        self.replay_block_prefix(&mut env, target, target_kind, replay, |evm, tx| {
            let mut ext = factory.evm_ext(&evm_env(evm));
            context.prepare_next(&mut ext, tx);
            ext.set_protocol_replay(ChainFor::<MonadEvmNetwork>::is_protocol_transaction(tx));
            *evm.ext_mut() = ext;
            context.record_transaction(tx.clone());
        })?;
        let mut ext = factory.evm_ext(&env);
        context.prepare_next(&mut ext, target);
        ext.set_protocol_replay(true);
        self.transact_tx(env, target, Some(ext))
    }
}

/// A transaction executed by the [`Executor`].
struct Execution<FEN: FoundryEvmNetwork> {
    /// The transaction result.
    result: TxResultFor<FEN>,
    /// The transaction's state changes.
    state: PendingState,
    /// The environment after the transaction.
    env: EvmEnvFor<FEN>,
    /// Whether the inspectors halted the execution due to cancellation.
    cancelled: bool,
}

/// Executes `tx` and returns its result and EVM for the caller to commit, park or discard.
fn execute<FEN: FoundryEvmNetwork>(
    backend: &mut dyn CheatcodesBackend<FEN>,
    env: EvmEnvFor<FEN>,
    tx: &Recovered<TxFor<FEN>>,
    ext: Option<ChainFor<FEN>>,
    stack: &mut InspectorStack<FEN>,
) -> eyre::Result<(Execution<FEN>, FoundryEvmFor<'static, FEN>)> {
    let sancov_edges = stack.inner.sancov_edges;
    let sancov_trace_cmp = stack.inner.sancov_trace_cmp;
    let mut evm = backend.backend().create_evm();
    install_env::<FEN>(backend.backend(), &mut evm, env);
    match ext {
        Some(ext) => *evm.ext_mut() = ext,
        None => evm.ext_mut().for_next_transaction(tx),
    }
    let executed = {
        let _guard = (sancov_edges || sancov_trace_cmp)
            .then(|| SancovGuard::new(sancov_edges, sancov_trace_cmp));
        evm.set_inspector(stack.as_inspector(&mut *backend));
        evm.transact(tx).map(|executed| executed.detach())
    };
    // SAFETY: The EVM was created by the backend's lifecycle over its owned databases.
    let evm: Evm<'_, _> = unsafe { release_inspector(evm) };
    let TxResultWithState { result, pending_state, .. } = executed.map_err(handler_error)?;

    // A spec or chain ID change deferred by a cheatcode applies from the next transaction.
    let mut env = evm_env(&evm);
    if let Some(pending) = backend.backend().lifecycle().pending_env() {
        env = EvmEnv::new(pending.spec, pending.version, env.block);
    }
    if let Some(spec) = stack.cheatcodes.as_deref().and_then(|cheats| cheats.execution_evm_version)
        && spec != env.spec
    {
        env = EvmFactoryFor::<FEN>::default().env_with_spec(env, spec);
    }
    Ok((
        Execution { result, state: pending_state, env, cancelled: stack.execution_cancelled() },
        evm,
    ))
}

/// Returns the default environment of `FEN`.
fn default_env<FEN: FoundryEvmNetwork>() -> EvmEnvFor<FEN> {
    let spec = SpecFor::<FEN>::default();
    let chain_id = foundry_common::DEV_CHAIN_ID;
    EvmEnv::new(spec, EvmFactoryFor::<FEN>::default().version(spec, chain_id), Default::default())
}

/// Builds an executor over an empty local backend with the default environment of `FEN`.
#[cfg(test)]
pub(crate) fn test_executor<FEN: FoundryEvmNetwork>(
    builder: ExecutorBuilder<FEN>,
    networks: NetworkConfigs,
) -> Executor<FEN> {
    let env = default_env::<FEN>();
    builder.build(env, Default::default(), Backend::spawn(None, env).unwrap(), networks)
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
    pub exit_reason: Option<InstrStop>,
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
    /// The state changes of the call: every account and storage slot it loaded, with their
    /// original and present values.
    pub state_changeset: PendingState,
    /// The environment after the call
    pub evm_env: EvmEnvFor<FEN>,
    /// The cheatcode states after execution
    pub cheatcodes: Option<Box<Cheatcodes<FEN>>>,
    /// The address of the contract a successful create deployed.
    pub created_address: Option<Address>,
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
    /// Owned post-call backend, excluding `state_changeset`. Adopted only on commit.
    pub(crate) post_execution_backend: Option<Box<Backend<FEN>>>,
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
            state_changeset: PendingState::default(),
            evm_env: default_env::<FEN>(),
            cheatcodes: Default::default(),
            created_address: None,
            fork_block_number: None,
            chisel_state: None,
            reverter: None,
            skip_payloads: Vec::new(),
            post_execution_backend: None,
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
            && reason.is_success()
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

    /// Sets the stipend to the intrinsic gas of `tx` in `evm_env`, with the access list cheatcodes
    /// applied to it, as on master.
    fn set_stipend(&mut self, evm_env: &EvmEnvFor<FEN>, mut tx: TransactionRequestFor<FEN>) {
        if let Some(access_list) =
            self.cheatcodes.as_ref().and_then(|cheats| cheats.applied_access_list.clone())
        {
            tx.set_access_list(access_list);
        }
        self.stipend = calculate_stipend::<FEN>(evm_env, &tx);
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

/// Returns the intrinsic gas `tx` pays before execution in `evm_env`.
pub fn calculate_stipend<FEN: FoundryEvmNetwork>(
    evm_env: &EvmEnvFor<FEN>,
    tx: &TransactionRequestFor<FEN>,
) -> u64 {
    let access_list = tx.access_list();
    evm2::ethereum::intrinsic_and_floor_gas(
        &evm_env.version,
        tx.from().unwrap_or_default(),
        tx.kind().unwrap_or(TxKind::Create),
        tx.input().unwrap_or_default(),
        access_list.map_or(0, |list| list.len() as u64),
        access_list.map_or(0, |list| list.iter().map(|item| item.storage_keys.len() as u64).sum()),
        tx.value().unwrap_or_default(),
    )
    .0
}

/// Converts the data aggregated in the `inspector` and `execution` to a `RawCallResult`.
fn convert_executed_result<FEN: FoundryEvmNetwork>(
    execution: Execution<FEN>,
    mut inspector: InspectorStack<FEN>,
    backend: &Backend<FEN>,
) -> eyre::Result<RawCallResult<FEN>> {
    let Execution { result: tx_result, state: state_changeset, env: evm_env, cancelled } =
        execution;
    let exit_reason = tx_result.stop;
    let gas_refunded = if tx_result.status { tx_result.refunded } else { 0 };
    let gas_used = tx_result.tx_gas_used();
    let created_address = tx_result.created_address.filter(|_| tx_result.status);
    let result = if created_address.is_some() { Bytes::new() } else { tx_result.output };

    let sancov_edges = inspector.inner.sancov_edges;
    let sancov_trace_cmp = inspector.inner.sancov_trace_cmp;
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
        .or_else(|| backend.active_fork_block_number());
    let debug_bytecodes = collect_debug_bytecodes(traces.as_ref(), backend, &state_changeset);

    if logs.is_empty() {
        logs = tx_result.logs;
    }

    let transactions = cheatcodes
        .as_ref()
        .map(|c| c.broadcastable_transactions.clone())
        .filter(|txs| !txs.is_empty());
    let skip_payloads =
        cheatcodes.as_mut().map(|c| std::mem::take(&mut c.skip_payloads)).unwrap_or_default();

    let mut raw = RawCallResult {
        exit_reason: Some(exit_reason),
        execution_cancelled: cancelled,
        reverted: !exit_reason.is_success(),
        has_state_snapshot_failure: backend.has_state_snapshot_failure(),
        result,
        gas_used,
        gas_refunded,
        stipend: 0,
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
        cheatcodes,
        created_address,
        fork_block_number,
        chisel_state,
        reverter,
        skip_payloads,
        post_execution_backend: None,
    };
    if sancov_edges {
        SancovGuard::append_edges_into(&mut raw);
    }
    if sancov_trace_cmp {
        SancovGuard::drain_cmp_into(&mut raw);
    }
    Ok(raw)
}

/// Returns the runtime bytecode of every traced account, as of after the call.
fn collect_debug_bytecodes<FEN: FoundryEvmNetwork>(
    traces: Option<&SparsedTraceArena>,
    backend: &Backend<FEN>,
    state: &PendingState,
) -> AddressHashMap<Bytes> {
    let mut bytecodes = HashMap::default();
    let Some(traces) = traces else { return bytecodes };

    for node in traces.arena.nodes() {
        let address = node.trace.address;
        if bytecodes.contains_key(&address) {
            continue;
        }

        let account = match state.account_info(&address) {
            Some(account) => account.clone(),
            None => {
                let Ok(Some(account)) = backend.basic(address) else { continue };
                account
            }
        };
        let code = match account.code {
            Some(code) if !code.is_empty() => Some(code),
            _ => backend.code_by_hash(account.code_hash).ok(),
        };
        let code = code.map(|code| code.original_bytes()).unwrap_or_default();
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

    /// Returns whether a campaign stop was requested or its deadline was observed.
    pub(crate) fn stop_requested(&self) -> bool {
        matches!(self, Self::Campaign { stop, .. } if stop.load(Ordering::Relaxed))
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

/// Derives the deterministic RNG seed for a fuzz or invariant worker.
///
/// Worker 0 keeps the configured seed, so a single worker replays the same sequence.
pub(crate) fn worker_seed(seed: U256, worker_id: u32) -> U256 {
    if worker_id == 0 {
        seed
    } else {
        let seed_data = [&seed.to_be_bytes::<32>()[..], &worker_id.to_be_bytes()[..]].concat();
        keccak256(seed_data).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspectors::{EdgeCovHit, EdgeKey};
    use alloy_eips::eip4788::SYSTEM_ADDRESS;
    use alloy_primitives::{address, bytes};
    use alloy_rpc_types::TransactionRequest;
    use evm2::SpecId;
    use foundry_cheatcodes::{
        CheatsConfig,
        Vm::{
            broadcastRawTransactionCall, loadCall, mockCallRevert_1Call, revertToStateCall,
            snapshotStateCall, storeCall,
        },
    };
    use foundry_config::Config;
    use foundry_evm_core::{constants::MAGIC_SKIP, evm::TempoEvmNetwork, opts::EvmOpts};
    use foundry_evm_traces::InternalTraceMode;
    use std::{sync::mpsc, thread};

    #[cfg(feature = "monad")]
    use foundry_evm_core::constants::MONAD_CHEATCODE_ADDRESS;

    fn eth_executor(builder: ExecutorBuilder<EthEvmNetwork>) -> Executor<EthEvmNetwork> {
        test_executor(builder, NetworkConfigs::default())
    }

    fn cheats_config() -> Arc<CheatsConfig> {
        Arc::new(CheatsConfig::new(&Config::default(), EvmOpts::default(), None, None, false))
    }

    /// Returns a fee-less legacy transaction from [`CALLER`].
    fn replay_tx(
        nonce: u64,
        kind: TxKind,
        input: &'static [u8],
    ) -> Recovered<TxFor<EthEvmNetwork>> {
        let mut request = TransactionRequest::default()
            .from(CALLER)
            .nonce(nonce)
            .gas_limit(100_000)
            .gas_price(0)
            .input(Bytes::from_static(input).into());
        request.set_kind(kind);
        EthEvmNetwork::tx_from_request(request).unwrap()
    }

    fn dense_call(edge: EdgeKey) -> RawCallResult {
        RawCallResult {
            edge_coverage: Some(EdgeCoverage::CollisionFree(vec![EdgeCovHit { edge, count: 1 }])),
            ..Default::default()
        }
    }

    #[test]
    fn nested_revert_is_ignored_only_when_allowed() {
        let target = Address::repeat_byte(0x11);
        let nested = Address::repeat_byte(0x22);

        assert!(should_ignore_revert(false, target, Some(nested), &[]));
        assert!(!should_ignore_revert(true, target, Some(nested), &[]));
        assert!(!should_ignore_revert(false, target, Some(target), &[]));
        assert!(!should_ignore_revert(false, target, Some(CHEATCODE_ADDRESS), &[]));
        assert!(!should_ignore_revert(false, target, None, &[]));
    }

    #[cfg(feature = "monad")]
    #[test]
    fn network_cheatcode_revert_handling_is_monad_specific() {
        let target = Address::repeat_byte(0x11);

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
        let ethereum =
            test_executor(ExecutorBuilder::<EthEvmNetwork>::new(), NetworkConfigs::with_monad());
        assert!(ethereum.backend().networks().is_monad());
        assert!(!ethereum.backend().is_persistent(&MONAD_CHEATCODE_ADDRESS));

        let monad =
            test_executor(ExecutorBuilder::<MonadEvmNetwork>::new(), NetworkConfigs::with_monad());
        assert!(monad.backend().networks().is_monad());
        assert!(monad.backend().is_persistent(&MONAD_CHEATCODE_ADDRESS));
    }

    #[test]
    fn tempo_labels_follow_concrete_builder() {
        let ethereum =
            test_executor(ExecutorBuilder::<EthEvmNetwork>::new(), NetworkConfigs::with_tempo());
        assert!(ethereum.inspector().tempo_labels.is_none());

        let tempo =
            test_executor(ExecutorBuilder::<TempoEvmNetwork>::new(), NetworkConfigs::default());
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
    fn block_replay_commits_prefix_and_traces_only_target() {
        let mut executor = eth_executor(ExecutorBuilder::default().gas_limit(1 << 20));
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
        let prefix = replay_tx(0, TxKind::Call(address), &[]);
        let reverted_create = replay_tx(1, TxKind::Create, &[0x5f, 0x5f, 0xfd]);
        let target = replay_tx(2, TxKind::Call(address), &[]);

        let env = *executor.evm_env();
        let result = executor
            .transact_with_block_replay(
                env,
                &target,
                TxKind::Call(address),
                &[(B256::repeat_byte(1), prefix), (B256::repeat_byte(2), reverted_create)],
            )
            .unwrap();

        assert_eq!(result.result, Bytes::from(B256::with_last_byte(2).0));
        assert_eq!(executor.get_nonce(CALLER).unwrap(), 3);
        assert_eq!(executor.backend().storage(address, U256::ZERO).unwrap(), U256::from(2));
        assert_eq!(result.traces.unwrap().arena.nodes().len(), 1);
    }

    #[test]
    fn block_replay_initializes_create_target_from_canonical_nonce() {
        let mut executor = eth_executor(ExecutorBuilder::default().gas_limit(1 << 20));
        executor.set_balance(CALLER, U256::MAX).unwrap();

        let prefix = replay_tx(0, TxKind::Call(Address::repeat_byte(0x11)), &[]);
        let target = replay_tx(1, TxKind::Create, &[0x00]);
        let expected = CALLER.create(1);

        let env = *executor.evm_env();
        let result = executor
            .transact_with_block_replay(
                env,
                &target,
                TxKind::Create,
                &[(B256::repeat_byte(1), prefix)],
            )
            .unwrap();

        assert_eq!(result.created_address, Some(expected));
        assert!(executor.backend().is_persistent(&expected));
        assert!(executor.backend().has_cheatcode_access(&expected));
    }

    #[test]
    fn block_replay_preserves_successful_prefix_deployment() {
        let mut executor = eth_executor(ExecutorBuilder::default().gas_limit(1 << 20));
        executor.set_balance(CALLER, U256::MAX).unwrap();

        let deployed = CALLER.create(0);
        let prefix = replay_tx(0, TxKind::Create, &[0x00]);
        let target = replay_tx(1, TxKind::Call(deployed), &[]);

        let env = *executor.evm_env();
        executor
            .transact_with_block_replay(
                env,
                &target,
                TxKind::Call(deployed),
                &[(B256::repeat_byte(1), prefix)],
            )
            .unwrap();

        assert!(executor.backend().is_persistent(&deployed));
    }

    #[cfg(feature = "monad")]
    #[test]
    fn monad_block_replay_executes_canonical_protocol_transactions() {
        const STAKING: Address =
            alloy_primitives::address!("0x0000000000000000000000000000000000001000");
        let system = foundry_common::MONAD_SYSTEM_ADDRESS;
        let system_tx = |nonce, gas_limit, input: Vec<u8>| {
            Recovered::new_unchecked(
                evm2::ethereum::TxEnvelope::Legacy(alloy_consensus::TxLegacy {
                    nonce,
                    gas_limit,
                    to: TxKind::Call(STAKING),
                    input: input.into(),
                    ..Default::default()
                }),
                system,
            )
        };
        let snapshot = system_tx(0, 0, keccak256("syscallSnapshot()")[..4].to_vec());
        let gas_paying = system_tx(1, 100_000, keccak256("syscallSnapshot()")[..4].to_vec());
        let epoch_change = system_tx(
            2,
            0,
            [&keccak256("syscallOnEpochChange(uint64)")[..4], &B256::with_last_byte(1)[..]]
                .concat(),
        );
        let mut executor = test_executor(
            ExecutorBuilder::<MonadEvmNetwork>::new().gas_limit(1 << 20),
            NetworkConfigs::with_monad(),
        );
        // Historical staking state is funded; EIP-161 prunes a touched empty account's storage.
        executor.set_balance(STAKING, U256::ONE).unwrap();
        let env = *executor.evm_env();
        let context = || BlockContext::new(Vec::new(), Vec::new(), Vec::new());

        // The canonical prefix syscall replays as a protocol call, the gas-paying system envelope
        // the caller selected runs as an ordinary transaction, and the canonical target replays
        // at the prefix's nonce.
        let result = executor
            .transact_with_monad_block_replay(
                env,
                &epoch_change,
                TxKind::Call(STAKING),
                &[(B256::repeat_byte(1), snapshot), (B256::repeat_byte(2), gas_paying.clone())],
                context(),
            )
            .unwrap();
        assert!(!result.reverted);
        assert_eq!(result.gas_used, 0);
        assert_eq!(executor.get_nonce(system).unwrap(), 3);
        assert_eq!(executor.get_balance(system).unwrap(), U256::ZERO);
        assert_eq!(executor.get_balance(STAKING).unwrap(), U256::ONE);
        let epoch = U256::from(1) << 192;
        assert_eq!(executor.backend().storage(STAKING, U256::from(1)).unwrap(), epoch);
        assert_eq!(executor.backend().storage(STAKING, U256::from(2)).unwrap(), U256::ZERO);

        // Reusing the accepted backend must not retain permission to replay protocol envelopes.
        let mut evm = executor.backend().create_evm();
        let next_snapshot = system_tx(3, 0, keccak256("syscallSnapshot()")[..4].to_vec());
        assert!(matches!(
            evm.transact(&next_snapshot),
            Err(evm2::registry::HandlerError::IntrinsicGasTooLow { .. })
        ));

        // A non-canonical target from the system sender is rejected without effects.
        let error = executor
            .transact_with_monad_block_replay(
                env,
                &gas_paying,
                TxKind::Call(STAKING),
                &[],
                context(),
            )
            .unwrap_err();
        assert_eq!(
            error.root_cause().to_string(),
            "invalid Monad protocol system transaction: gas limit must be zero, got 100000"
        );
        assert_eq!(executor.get_nonce(system).unwrap(), 3);
    }

    #[test]
    fn set_spec_id_updates_spec_dependent_cfg_state() {
        let mut executor = eth_executor(ExecutorBuilder::default().spec_id(SpecId::HOMESTEAD));
        assert_eq!(
            executor.evm_env().version.gas_params,
            evm2::Version::new(SpecId::HOMESTEAD).gas_params
        );
        assert!(!executor.evm_env().version.feature(evm2::EvmFeatures::EIP8037));

        executor.set_spec_id(SpecId::AMSTERDAM);

        assert_eq!(executor.spec_id(), SpecId::AMSTERDAM);
        assert_eq!(
            executor.evm_env().version.gas_params,
            evm2::Version::new(SpecId::AMSTERDAM).gas_params
        );
        assert!(executor.evm_env().version.feature(evm2::EvmFeatures::EIP8037));
    }

    #[test]
    fn calculate_stipend_uses_eip2780_transaction_context() {
        let caller = Address::repeat_byte(0x11);
        let recipient = Address::repeat_byte(0x22);
        let mut env = EvmFactoryFor::<EthEvmNetwork>::default()
            .env_with_spec(default_env::<EthEvmNetwork>(), SpecId::AMSTERDAM);
        let tx = TransactionRequest::default().from(caller).to(recipient);

        // EIP-2780 base cost plus the EIP-8038 cold recipient access.
        assert_eq!(calculate_stipend::<EthEvmNetwork>(&env, &tx), 15_000);
        // A self-transfer pays no recipient access.
        let self_transfer = tx.clone().to(caller);
        assert_eq!(calculate_stipend::<EthEvmNetwork>(&env, &self_transfer), 12_000);

        env = EvmFactoryFor::<EthEvmNetwork>::default().env_with_spec(env, SpecId::OSAKA);
        assert_eq!(calculate_stipend::<EthEvmNetwork>(&env, &tx), 21_000);
    }

    #[test]
    fn amsterdam_intercepted_create_refunds_state_gas() {
        let mut executor = eth_executor(
            ExecutorBuilder::default()
                .inspectors(|stack| stack.cheatcodes(cheats_config()))
                .spec_id(SpecId::AMSTERDAM)
                .gas_limit(1_000_000),
        );

        let target = Address::repeat_byte(0x11);
        // PUSH0; PUSH0; PUSH0; CREATE; POP; STOP.
        executor
            .set_code(
                target,
                Bytecode::new_raw(Bytes::from_static(&[0x5f, 0x5f, 0x5f, 0xf0, 0x50, 0x00])),
            )
            .unwrap();
        executor.inspector_mut().cheatcodes.as_mut().unwrap().core.intercept_next_create_call =
            true;

        let result = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();

        assert!(!result.reverted);
        // EIP-8037 account creation costs 183600 state gas.
        assert!(
            result.gas_used < 183_600,
            "failed CREATE retained its conditional state-gas charge"
        );
    }

    #[test]
    fn amsterdam_mocked_call_revert_refunds_state_gas() {
        let mut executor = eth_executor(
            ExecutorBuilder::default()
                .inspectors(|stack| stack.cheatcodes(cheats_config()))
                .spec_id(SpecId::AMSTERDAM)
                .gas_limit(1_000_000),
        );

        let target = Address::repeat_byte(0x11);
        let mocked = Address::repeat_byte(0x22);
        let mock = executor
            .transact_raw(
                CALLER,
                CHEATCODE_ADDRESS,
                mockCallRevert_1Call {
                    callee: mocked,
                    msgValue: U256::ONE,
                    data: Bytes::new(),
                    revertData: Bytes::new(),
                }
                .abi_encode()
                .into(),
                U256::ZERO,
            )
            .unwrap();
        assert!(!mock.reverted);
        executor.set_code(mocked, Bytecode::default()).unwrap();
        executor.set_balance(target, U256::ONE).unwrap();

        // PUSH0 x4; PUSH1 1; PUSH20 <mocked>; GAS; CALL; POP; STOP.
        let mut code = vec![0x5f, 0x5f, 0x5f, 0x5f, 0x60, 0x01, 0x73];
        code.extend_from_slice(mocked.as_slice());
        code.extend_from_slice(&[0x5a, 0xf1, 0x50, 0x00]);
        executor.set_code(target, Bytecode::new_raw(code.into())).unwrap();

        let result = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();

        assert!(!result.reverted);
        // EIP-8037 account creation costs 183600 state gas.
        assert!(
            result.gas_used < 183_600,
            "reverted mocked CALL retained its conditional state-gas charge"
        );
    }

    #[test]
    fn amsterdam_deploy_charges_create_state_gas_at_caller_nonce() {
        let mut executor = eth_executor(
            ExecutorBuilder::default().spec_id(SpecId::AMSTERDAM).gas_limit(1_000_000),
        );
        let sender = Address::repeat_byte(0x33);
        executor.set_account_nonce(sender, 5).unwrap();
        // A non-empty address for nonce 0 must not hide the charge for the address for nonce 5.
        executor.set_balance(sender.create(0), U256::ONE).unwrap();

        let result = executor.deploy(sender, Bytes::new(), U256::ZERO, None).unwrap();

        assert_eq!(result.address, sender.create(5));
        // An empty creation pays 24000 regular gas plus 183600 EIP-8037 account-creation gas.
        assert_eq!(result.raw.gas_used, 207_600);
    }

    #[test]
    fn set_trace_requirements_replaces_trace_mode_between_transactions() {
        let mut executor = eth_executor(ExecutorBuilder::default().gas_limit(1 << 20));
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
        let mut executor = eth_executor(ExecutorBuilder::default().gas_limit(GAS_LIMIT));
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
        assert_eq!(result.exit_reason, Some(InstrStop::Stop));
        assert!(result.gas_used > 21_000, "interrupt fired before EVM execution started");
        assert!(result.gas_used < GAS_LIMIT, "execution ran out of gas instead of exiting");
    }

    #[test]
    fn completed_execution_is_not_retroactively_cancelled() {
        let mut executor = eth_executor(ExecutorBuilder::default().gas_limit(1 << 24));
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
        let mut executor = eth_executor(ExecutorBuilder::default().gas_limit(GAS_LIMIT));
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
        assert_eq!(result.exit_reason, Some(InstrStop::Stop));
        assert!(result.gas_used < GAS_LIMIT, "execution ran out of gas instead of timing out");
    }

    #[test]
    fn campaign_deadline_interrupts_call_recursion_without_jumps() {
        let mut executor = eth_executor(ExecutorBuilder::default().gas_limit(1 << 24));
        let cancellation = EvmExecutionCancellation::campaign(
            EarlyExit::new(false),
            Arc::new(AtomicBool::new(false)),
            Some(Instant::now()),
        );
        executor.inspector_mut().set_execution_cancellation(cancellation);

        let target = Address::repeat_byte(0x11);
        // PUSH0 x5; ADDRESS; GAS; CALL; STOP calls itself until it runs out of gas.
        executor
            .set_code(
                target,
                Bytecode::new_raw(Bytes::from_static(&[
                    0x5f, 0x5f, 0x5f, 0x5f, 0x5f, 0x30, 0x5a, 0xf1, 0x00,
                ])),
            )
            .unwrap();

        let result = executor.transact_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        assert!(result.execution_cancelled);
        assert_eq!(result.exit_reason, Some(InstrStop::Stop));
    }

    #[test]
    fn gas_price_opcode_reads_configured_gas_price() {
        let mut env = default_env::<EthEvmNetwork>();
        env.set_chain_id(1);
        let mut executor = ExecutorBuilder::<EthEvmNetwork>::default()
            .inspectors(|stack| stack.cheatcodes(cheats_config()))
            .gas_limit(1 << 20)
            .build(
                env,
                TransactionRequest::default().gas_price(7),
                Backend::spawn(None, env).unwrap(),
                NetworkConfigs::default(),
            );
        let target = Address::repeat_byte(0x11);
        // GASPRICE; PUSH0; MSTORE; PUSH1 32; PUSH0; RETURN.
        executor
            .set_code(
                target,
                Bytecode::new_raw(Bytes::from_static(&[0x3a, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3])),
            )
            .unwrap();

        // Executions are fee-less, but the opcode reads the configured price.
        let result = executor.call_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        assert_eq!(result.result, Bytes::from(B256::with_last_byte(7).0));

        let sender = address!("5316812db67073c4d4af8bb3000c5b86c2877e94");
        let replay_target = address!("6fd0a0cff9a87adf51695b40b4fa267855a8f4c6");
        executor.set_balance(sender, U256::from(10u64.pow(18))).unwrap();
        // GASPRICE; PUSH0; SSTORE; STOP.
        executor.set_code(replay_target, Bytecode::new_raw(bytes!("3a5f5500"))).unwrap();
        // The signed transaction pays 100 wei per gas, independently of the configured price.
        let input = broadcastRawTransactionCall {
            data: bytes!("f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856"),
        }
        .abi_encode()
        .into();
        let result = executor.transact_raw(CALLER, CHEATCODE_ADDRESS, input, U256::ZERO).unwrap();
        assert!(!result.reverted, "{result:?}");
        assert_eq!(executor.backend().storage(replay_target, U256::ZERO).unwrap(), U256::from(100));

        // The replay does not consume the outer transaction's configured price.
        let result = executor.call_raw(CALLER, target, Bytes::new(), U256::ZERO).unwrap();
        assert_eq!(result.result, Bytes::from(B256::with_last_byte(7).0));
    }

    #[test]
    fn beacon_root_system_call_does_not_persist_system_address() {
        let mut executor = eth_executor(ExecutorBuilder::default().spec_id(SpecId::CANCUN));
        let before = executor.backend().basic(SYSTEM_ADDRESS).unwrap();
        let root = B256::repeat_byte(0x11);

        executor.apply_beacon_root(root).unwrap();

        assert_eq!(
            executor.backend().basic(SYSTEM_ADDRESS).unwrap(),
            before,
            "EIP-4788 system calls must not persist the system caller account",
        );
    }

    #[test]
    fn storage_stored_on_absent_account_survives_commit() {
        let mut executor = eth_executor(
            ExecutorBuilder::default()
                .inspectors(|stack| stack.cheatcodes(cheats_config()))
                .gas_limit(1_000_000),
        );
        let target = Address::with_last_byte(0x31);
        let value = B256::with_last_byte(1);
        let store = storeCall { target, slot: B256::ZERO, value }.abi_encode();
        let stored = executor.transact_raw(CALLER, CHEATCODE_ADDRESS, store.into(), U256::ZERO);
        assert!(!stored.unwrap().reverted);

        let load = loadCall { target, slot: B256::ZERO }.abi_encode();
        let loaded = executor.call_raw(CALLER, CHEATCODE_ADDRESS, load.into(), U256::ZERO).unwrap();
        assert_eq!(loaded.result, Bytes::from(value.0));
    }

    #[test]
    fn worker_seed_preserves_master_seed_and_derives_workers() {
        let seed = U256::from(0x1234);

        assert_eq!(worker_seed(seed, 0), seed);
        assert_ne!(worker_seed(seed, 1), seed);
        assert_ne!(worker_seed(seed, 1), worker_seed(seed, 2));
        assert_ne!(worker_seed(seed, 1), worker_seed(U256::from(0x5678), 1));
    }

    #[test]
    fn call_commit_preserves_lifecycle_state() {
        let mut executor = eth_executor(
            ExecutorBuilder::default()
                .inspectors(|stack| stack.cheatcodes(cheats_config()))
                .gas_limit(1_000_000),
        );
        let target = Address::with_last_byte(0x31);
        executor.set_balance(target, U256::ONE).unwrap();
        // The cheatcode inspector cancels root-call nonce increments.
        executor.set_account_nonce(CALLER, 5).unwrap();
        let snapshot = snapshotStateCall {}.abi_encode();
        let restore = revertToStateCall { snapshotId: U256::ZERO }.abi_encode();

        // Discarded calls must not retain snapshots.
        executor.call_raw(CALLER, CHEATCODE_ADDRESS, snapshot.clone().into(), U256::ZERO).unwrap();
        let missing = executor
            .call_raw(CALLER, CHEATCODE_ADDRESS, restore.clone().into(), U256::ZERO)
            .unwrap();
        assert_eq!(U256::from_be_slice(&missing.result), U256::ZERO);

        let mut captured =
            executor.call_raw(CALLER, CHEATCODE_ADDRESS, snapshot.into(), U256::ZERO).unwrap();
        executor.commit(&mut captured);
        let store = storeCall { target, slot: B256::ZERO, value: B256::with_last_byte(7) };
        let mut written = executor
            .call_raw(CALLER, CHEATCODE_ADDRESS, store.abi_encode().into(), U256::ZERO)
            .unwrap();
        executor.commit(&mut written);
        assert_eq!(executor.backend().storage(target, U256::ZERO).unwrap(), U256::from(7));

        let mut restored =
            executor.call_raw(CALLER, CHEATCODE_ADDRESS, restore.into(), U256::ZERO).unwrap();
        assert_eq!(U256::from_be_slice(&restored.result), U256::ONE);
        // Restored state stays detached until commit.
        assert_eq!(executor.backend().storage(target, U256::ZERO).unwrap(), U256::from(7));
        executor.commit(&mut restored);
        assert_eq!(executor.backend().storage(target, U256::ZERO).unwrap(), U256::ZERO);
        assert_eq!(executor.get_nonce(CALLER).unwrap(), 5);
    }

    #[test]
    fn call_success_checks_restored_assertion_state() {
        for legacy in [false, true] {
            let mut executor = eth_executor(
                ExecutorBuilder::default()
                    .inspectors(|stack| stack.cheatcodes(cheats_config()))
                    .legacy_assertions(legacy)
                    .gas_limit(1_000_000),
            );
            let target = Address::with_last_byte(0x31);
            // DSTest-style failed(): return storage slot zero.
            let code = Bytes::from_static(&[0x5f, 0x54, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3]);
            executor.set_code(target, Bytecode::new_raw(code)).unwrap();
            let (account, slot) =
                if legacy { (target, U256::ZERO) } else { (CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT) };
            executor.backend_mut().insert_account_storage(account, slot, U256::ONE);
            let snapshot = snapshotStateCall {}.abi_encode();
            let captured = executor
                .transact_raw(CALLER, CHEATCODE_ADDRESS, snapshot.into(), U256::ZERO)
                .unwrap();
            let restore = revertToStateCall { snapshotId: U256::from_be_slice(&captured.result) }
                .abi_encode();
            executor.backend_mut().insert_account_storage(account, slot, U256::ZERO);

            let mut restored =
                executor.call_raw(CALLER, CHEATCODE_ADDRESS, restore.into(), U256::ZERO).unwrap();
            assert!(!restored.reverted);
            // Legacy failed() ignores committed flags; the global check includes them.
            let pending = Cow::Borrowed(&restored.state_changeset);
            assert_eq!(executor.is_raw_call_success(target, pending, &restored), legacy);
            executor.commit(&mut restored);
            assert_eq!(executor.is_raw_call_mut_success(target, &mut restored), legacy);
        }
    }
}
