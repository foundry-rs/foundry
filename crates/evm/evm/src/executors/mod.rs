//! EVM executor abstractions, which can execute calls.
//!
//! Used for running tests, scripts, and interacting with the inner backend which holds the state.

// TODO: The individual executors in this module should be moved into the respective crates, and the
// `Executor` struct should be accessed using a trait defined in `foundry-evm-core` instead of
// the concrete `Executor` type.

use crate::inspectors::{
    Cheatcodes, InspectorData, InspectorStack, cheatcodes::BroadcastableTransactions,
};
use alloy_dyn_abi::{DynSolValue, FunctionExt, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{
    Address, Bytes, Log, TxKind, U256, keccak256,
    map::{AddressHashMap, HashMap},
};
use alloy_sol_types::{SolCall, sol};
use foundry_evm_core::{
    EvmEnv, FoundryBlock, FoundryTransaction,
    backend::{Backend, BackendError, BackendResult, CowBackend, DatabaseExt, GLOBAL_FAIL_SLOT},
    constants::{
        CALLER, CHEATCODE_ADDRESS, CHEATCODE_CONTRACT_HASH, DEFAULT_CREATE2_DEPLOYER,
        DEFAULT_CREATE2_DEPLOYER_CODE, DEFAULT_CREATE2_DEPLOYER_DEPLOYER,
    },
    decode::{RevertDecoder, SkipReason},
    evm::{
        EthEvmNetwork, EvmEnvFor, FoundryEvmNetwork, HaltReasonFor, IntoInstructionResult, SpecFor,
        TxEnvFor,
    },
    utils::StateChangeset,
};
use foundry_evm_coverage::HitMaps;
use foundry_evm_traces::{SparsedTraceArena, TraceMode};
use revm::{
    bytecode::Bytecode,
    context::Transaction,
    context_interface::{
        result::{ExecutionResult, Output, ResultAndState},
        transaction::SignedAuthorization,
    },
    database::{DatabaseCommit, DatabaseRef},
    interpreter::{InstructionResult, return_ok},
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

pub mod fuzz;
pub use fuzz::FuzzedExecutor;

pub mod invariant;
pub use invariant::InvariantExecutor;

mod corpus;
mod sancov;
mod trace;

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
}

impl<FEN: FoundryEvmNetwork> Executor<FEN> {
    /// Creates a new `Executor` with the given arguments.
    #[inline]
    pub fn new(
        mut backend: Backend<FEN>,
        evm_env: EvmEnvFor<FEN>,
        tx_env: TxEnvFor<FEN>,
        inspector: InspectorStack<FEN>,
        gas_limit: u64,
        legacy_assertions: bool,
    ) -> Self {
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

        Self {
            backend: Arc::new(backend),
            evm_env,
            tx_env,
            inspector,
            gas_limit,
            legacy_assertions,
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

    /// Sets the EVM spec.
    pub const fn set_spec_id(&mut self, spec_id: SpecFor<FEN>) {
        self.evm_env.cfg_env.spec = spec_id;
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

    /// Set the nonce of an account.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) -> BackendResult<()> {
        let mut account = self.backend().basic_ref(address)?.unwrap_or_default();
        account.nonce = nonce;
        self.backend_mut().insert_account_info(address, account);
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

    /// Returns `true` if the account has no code.
    pub fn is_empty_code(&self, address: Address) -> BackendResult<bool> {
        Ok(self.backend().basic_ref(address)?.map(|acc| acc.is_empty_code_hash()).unwrap_or(true))
    }

    #[inline]
    pub fn set_tracing(&mut self, mode: TraceMode) -> &mut Self {
        self.inspector_mut().tracing(mode);
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
        assert!(
            matches!(tx_env.kind(), TxKind::Create),
            "Expected create transaction, got {:?}",
            tx_env.kind()
        );
        trace!(sender=%tx_env.caller(), "deploying contract");

        let mut result = self.transact_with_env(evm_env, tx_env)?;
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

    /// Execute the transaction configured in `tx_env`.
    ///
    /// The state after the call is **not** persisted.
    #[instrument(name = "call", level = "debug", skip_all)]
    pub fn call_with_env(
        &self,
        mut evm_env: EvmEnvFor<FEN>,
        mut tx_env: TxEnvFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let mut stack = self.inspector().clone();
        let sancov_edges = stack.inner.sancov_edges;
        let sancov_trace_cmp = stack.inner.sancov_trace_cmp;
        let sancov_active = sancov_edges || sancov_trace_cmp;
        let mut backend = CowBackend::new_borrowed(self.backend());
        let result = {
            let _guard = sancov_active.then(|| SancovGuard::new(sancov_edges, sancov_trace_cmp));
            backend.inspect(&mut evm_env, &mut tx_env, &mut stack)?
        };
        let mut result = convert_executed_result(
            evm_env,
            tx_env,
            stack,
            result,
            backend.has_state_snapshot_failure(),
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
        mut evm_env: EvmEnvFor<FEN>,
        mut tx_env: TxEnvFor<FEN>,
    ) -> eyre::Result<RawCallResult<FEN>> {
        let mut stack = self.inspector().clone();
        let sancov_edges = stack.inner.sancov_edges;
        let sancov_trace_cmp = stack.inner.sancov_trace_cmp;
        let sancov_active = sancov_edges || sancov_trace_cmp;
        let backend = self.backend_mut();
        let result = {
            let _guard = sancov_active.then(|| SancovGuard::new(sancov_edges, sancov_trace_cmp));
            backend.inspect(&mut evm_env, &mut tx_env, &mut stack)?
        };
        let mut result = convert_executed_result(
            evm_env,
            tx_env,
            stack,
            result,
            backend.has_state_snapshot_failure(),
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
        let success = self.is_success_raw(address, reverted, state_changeset);
        should_fail ^ success
    }

    #[instrument(name = "is_success", level = "debug", skip_all)]
    fn is_success_raw(
        &self,
        address: Address,
        reverted: bool,
        state_changeset: Cow<'_, StateChangeset>,
    ) -> bool {
        // The call reverted.
        if reverted {
            return false;
        }

        // A failure occurred in a reverted snapshot, which is considered a failed test.
        if self.backend().has_state_snapshot_failure() {
            return false;
        }

        // Check the global failure slot.
        if self.has_global_failure(&state_changeset) {
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

    /// Clears the global assertion failure flag from both the committed backend state and, when
    /// provided, the in-flight state changeset for the current call.
    pub fn clear_global_failure(
        &mut self,
        state_changeset: Option<&mut StateChangeset>,
    ) -> BackendResult<()> {
        if let Some(state_changeset) = state_changeset
            && let Some(acc) = state_changeset.get_mut(&CHEATCODE_ADDRESS)
            && let Some(failed_slot) = acc.storage.get_mut(&GLOBAL_FAIL_SLOT)
        {
            failed_slot.present_value = U256::ZERO;
        }

        self.set_storage_slot(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT, U256::ZERO)
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
    /// The line coverage info collected during the call
    pub line_coverage: Option<HitMaps>,
    /// The edge coverage info collected during the call
    pub edge_coverage: Option<Vec<u8>>,
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
    /// The chisel state
    pub chisel_state: Option<(Vec<U256>, Vec<u8>)>,
    pub reverter: Option<Address>,
}

impl<FEN: FoundryEvmNetwork> Default for RawCallResult<FEN> {
    fn default() -> Self {
        Self {
            exit_reason: None,
            reverted: false,
            has_state_snapshot_failure: false,
            result: Bytes::new(),
            gas_used: 0,
            gas_refunded: 0,
            stipend: 0,
            logs: Vec::new(),
            labels: HashMap::default(),
            traces: None,
            line_coverage: None,
            edge_coverage: None,
            sancov_coverage: None,
            sancov_cmp_values: None,
            transactions: None,
            state_changeset: HashMap::default(),
            evm_env: EvmEnv::default(),
            tx_env: TxEnvFor::<FEN>::default(),
            cheatcodes: Default::default(),
            out: None,
            chisel_state: None,
            reverter: None,
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

    /// Converts the result of the call into an `EvmError`.
    pub fn into_evm_error(self, rd: Option<&RevertDecoder>) -> EvmError<FEN> {
        if self.reverter == Some(CHEATCODE_ADDRESS)
            && let Some(reason) = SkipReason::decode(&self.result)
        {
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
    /// Uses AFL binning algo <https://github.com/h0mbre/Lucid/blob/3026e7323c52b30b3cf12563954ac1eaa9c6981e/src/coverage.rs#L57-L85>
    pub fn merge_edge_coverage(&mut self, history_map: &mut [u8]) -> (bool, bool) {
        let mut new_coverage = false;
        let mut is_edge = false;
        if let Some(x) = &mut self.edge_coverage {
            // Iterate over the current map and the history map together and update
            // the history map, if we discover some new coverage, report true
            for (curr, hist) in std::iter::zip(x, history_map) {
                // If we got a hitcount of at least 1
                if *curr > 0 {
                    // Convert hitcount into bucket count
                    let bucket = match *curr {
                        0 => 0,
                        1 => 1,
                        2 => 2,
                        3 => 4,
                        4..=7 => 8,
                        8..=15 => 16,
                        16..=31 => 32,
                        32..=127 => 64,
                        128..=255 => 128,
                    };

                    // If the old record for this edge pair is lower, update
                    if *hist < bucket {
                        if *hist == 0 {
                            // Counts as an edge the first time we see it, otherwise it's a feature.
                            is_edge = true;
                        }
                        *hist = bucket;
                        new_coverage = true;
                    }

                    // Zero out the current map for next iteration.
                    *curr = 0;
                }
            }
        }
        (new_coverage, is_edge)
    }

    /// Update provided history map with sancov coverage info collected during this call.
    /// Same AFL binning algo as [`Self::merge_edge_coverage`].
    pub fn merge_sancov_coverage(&mut self, history_map: &mut Vec<u8>) -> (bool, bool) {
        let mut new_coverage = false;
        let mut is_edge = false;
        if let Some(x) = &mut self.sancov_coverage {
            if history_map.len() < x.len() {
                history_map.resize(x.len(), 0);
            }
            for (curr, hist) in std::iter::zip(x.iter_mut(), history_map.iter_mut()) {
                if *curr > 0 {
                    let bucket = match *curr {
                        0 => 0,
                        1 => 1,
                        2 => 2,
                        3 => 4,
                        4..=7 => 8,
                        8..=15 => 16,
                        16..=31 => 32,
                        32..=127 => 64,
                        128..=255 => 128,
                    };
                    if *hist < bucket {
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
        evm_history: &mut [u8],
        sancov_history: &mut Vec<u8>,
    ) -> (bool, bool) {
        let (new_evm, edge_evm) = self.merge_edge_coverage(evm_history);
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

/// Converts the data aggregated in the `inspector` and `call` to a `RawCallResult`
fn convert_executed_result<FEN: FoundryEvmNetwork>(
    evm_env: EvmEnvFor<FEN>,
    tx_env: TxEnvFor<FEN>,
    inspector: InspectorStack<FEN>,
    ResultAndState { result, state: state_changeset }: ResultAndState<HaltReasonFor<FEN>>,
    has_state_snapshot_failure: bool,
) -> eyre::Result<RawCallResult<FEN>> {
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
    let gas = revm::interpreter::gas::calculate_initial_tx_gas_for_tx(
        &tx_env,
        evm_env.cfg_env.spec.into(),
    );

    let result = match &out {
        Some(Output::Call(data)) => data.clone(),
        _ => Bytes::new(),
    };

    let InspectorData {
        mut logs,
        labels,
        traces,
        line_coverage,
        edge_coverage,
        cheatcodes,
        chisel_state,
        reverter,
    } = inspector.collect();

    if logs.is_empty() {
        logs = exec_logs;
    }

    let transactions = cheatcodes
        .as_ref()
        .map(|c| c.broadcastable_transactions.clone())
        .filter(|txs| !txs.is_empty());

    Ok(RawCallResult {
        exit_reason: Some(exit_reason),
        reverted: !matches!(exit_reason, return_ok!()),
        has_state_snapshot_failure,
        result,
        gas_used,
        gas_refunded,
        stipend: gas.initial_total_gas,
        logs,
        labels,
        traces,
        line_coverage,
        edge_coverage,
        sancov_coverage: None,
        sancov_cmp_values: None,
        transactions,
        state_changeset,
        evm_env,
        tx_env,
        cheatcodes,
        out,
        chisel_state,
        reverter,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm_core::constants::MAGIC_SKIP;

    #[test]
    fn cheatcode_skip_payload_is_classified_as_skip() {
        let raw = RawCallResult::<EthEvmNetwork> {
            result: Bytes::from_static(b"FOUNDRY::SKIPwith reason"),
            reverter: Some(CHEATCODE_ADDRESS),
            ..Default::default()
        };

        let err = raw.into_evm_error(None);
        assert!(matches!(err, EvmError::Skip(_)));
    }

    #[test]
    fn forged_skip_payload_from_non_cheatcode_is_execution_error() {
        let raw = RawCallResult::<EthEvmNetwork> {
            result: Bytes::from_static(MAGIC_SKIP),
            reverter: Some(CALLER),
            ..Default::default()
        };

        let err = raw.into_evm_error(None);
        assert!(matches!(err, EvmError::Execution(_)));
    }

    #[test]
    fn skip_payload_without_reverter_is_execution_error() {
        let raw = RawCallResult::<EthEvmNetwork> {
            result: Bytes::from_static(MAGIC_SKIP),
            reverter: None,
            ..Default::default()
        };

        let err = raw.into_evm_error(None);
        assert!(matches!(err, EvmError::Execution(_)));
    }
}
