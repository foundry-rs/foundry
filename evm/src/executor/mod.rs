use self::inspector::{
    cheatcodes::util::BroadcastableTransactions, Cheatcodes, InspectorData, InspectorStackConfig,
};
use crate::{debug::DebugArena, decode, trace::CallTraceArena, CALLER, utils::{h160_to_b160, b160_to_h160, eval_to_instruction_result, halt_to_instruction_result}};
pub use abi::{
    patch_hardhat_console_selector, HardhatConsoleCalls, CHEATCODE_ADDRESS, CONSOLE_ABI,
    HARDHAT_CONSOLE_ABI, HARDHAT_CONSOLE_ADDRESS,
};
use backend::FuzzBackendWrapper;
use bytes::Bytes;
use ethers::{
    abi::{Abi, Contract, Detokenize, Function, Tokenize},
    prelude::{decode_function_data, encode_function_data, Address, U256},
    signers::LocalWallet,
    types::Log,
};
use foundry_common::abi::IntoFunction;
use hashbrown::HashMap;
<<<<<<< HEAD
=======
use revm::{
    db::DatabaseCommit, primitives::{B160, ResultAndState}
};
>>>>>>> 44b13209 (chore: modify state changeset to use proper types, annoying type conversions remain)
/// Reexport commonly used revm types
pub use revm::{db::DatabaseRef, Env, SpecId};
use std::collections::BTreeMap;
use revm::interpreter::{CreateScheme, InstructionResult, Memory, return_ok, Stack};
use revm::primitives::{Account, BlockEnv, Bytecode, Env, ExecutionResult, SpecId, TransactTo, TxEnv, Output, U256 as rU256};
use tracing::trace;

/// ABIs used internally in the executor
pub mod abi;
/// custom revm database implementations
pub mod backend;
pub use backend::Backend;
/// Executor builder
pub mod builder;
/// Forking provider
pub mod fork;
/// Executor inspectors
pub mod inspector;
/// Executor configuration
pub mod opts;
pub mod snapshot;

use crate::{
    coverage::HitMaps,
    executor::{
        backend::{
            error::{DatabaseError, DatabaseResult},
            DatabaseExt,
        },
        inspector::{InspectorStack, DEFAULT_CREATE2_DEPLOYER},
    },
};
pub use builder::ExecutorBuilder;

/// A mapping of addresses to their changed state.
pub type StateChangeset = HashMap<B160, Account>;

/// A type that can execute calls
///
/// The executor can be configured with various `revm::Inspector`s, like `Cheatcodes`.
///
/// There are two ways of executing calls:
///  - `committing`: any state changes made during the call are recorded and are persisting
///  - `raw`: state changes only exist for the duration of the call and are discarded afterwards, in
///    other words: the state of the underlying database remains unchanged.
#[derive(Debug, Clone)]
pub struct Executor {
    /// The underlying `revm::Database` that contains the EVM storage
    // Note: We do not store an EVM here, since we are really
    // only interested in the database. REVM's `EVM` is a thin
    // wrapper around spawning a new EVM on every call anyway,
    // so the performance difference should be negligible.
    backend: Backend,
    env: Env,
    inspector_config: InspectorStackConfig,
    /// The gas limit for calls and deployments. This is different from the gas limit imposed by
    /// the passed in environment, as those limits are used by the EVM for certain opcodes like
    /// `gaslimit`.
    gas_limit: U256,
}

// === impl Executor ===

impl Executor {
    pub fn new(
        mut backend: Backend,
        env: Env,
        inspector_config: InspectorStackConfig,
        gas_limit: U256,
    ) -> Self {
        // Need to create a non-empty contract on the cheatcodes address so `extcodesize` checks
        // does not fail
        backend.insert_account_info(
            CHEATCODE_ADDRESS,
            revm::primitives::AccountInfo {
                code: Some(Bytecode::new_raw(vec![0u8].into()).to_checked()),
                ..Default::default()
            },
        );

        Executor { backend, env, inspector_config, gas_limit }
    }

    /// Returns a reference to the Env
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Returns a mutable reference to the Env
    pub fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    /// Returns a mutable reference to the Backend
    pub fn backend_mut(&mut self) -> &mut Backend {
        &mut self.backend
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Returns an immutable reference to the InspectorStackConfig
    pub fn inspector_config(&self) -> &InspectorStackConfig {
        &self.inspector_config
    }

    /// Returns a mutable reference to the InspectorStackConfig
    pub fn inspector_config_mut(&mut self) -> &mut InspectorStackConfig {
        &mut self.inspector_config
    }

    /// Creates the default CREATE2 Contract Deployer for local tests and scripts.
    pub fn deploy_create2_deployer(&mut self) -> eyre::Result<()> {
        trace!("deploying local create2 deployer");
        let create2_deployer_account = self
            .backend_mut()
            .basic(h160_to_b160(DEFAULT_CREATE2_DEPLOYER))?
            .ok_or(DatabaseError::MissingAccount(DEFAULT_CREATE2_DEPLOYER))?;

        if create2_deployer_account.code.is_none() ||
            create2_deployer_account.code.as_ref().unwrap().is_empty()
        {
            let creator = "0x3fAB184622Dc19b6109349B94811493BF2a45362".parse().unwrap();

            // Probably 0, but just in case.
            let initial_balance = self.get_balance(creator)?;

            self.set_balance(creator, U256::MAX)?;
            let res = self.deploy(
                creator,
                hex::decode("604580600e600039806000f350fe7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3").expect("valid hex").into(),
                U256::zero(),
                None
            )?;
            trace!(create2=?res.address, "deployed local create2 deployer");

            self.set_balance(creator, initial_balance)?;
        }
        Ok(())
    }

    /// Set the balance of an account.
    pub fn set_balance(&mut self, address: Address, amount: U256) -> DatabaseResult<&mut Self> {
        trace!(?address, ?amount, "setting account balance");
        let mut account = self.backend_mut().basic(h160_to_b160(address))?.unwrap_or_default();
        account.balance = amount.into();

        self.backend_mut().insert_account_info(address, account);
        Ok(self)
    }

    /// Gets the balance of an account
    pub fn get_balance(&self, address: Address) -> DatabaseResult<U256> {
        Ok(self.backend().basic(h160_to_b160(address))?.map(|acc| acc.balance.into()).unwrap_or_default())
    }

    /// Set the nonce of an account.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) -> DatabaseResult<&mut Self> {
        let mut account = self.backend_mut().basic(h160_to_b160(address))?.unwrap_or_default();
        account.nonce = nonce;

        self.backend_mut().insert_account_info(address, account);
        Ok(self)
    }

    pub fn set_tracing(&mut self, tracing: bool) -> &mut Self {
        self.inspector_config.tracing = tracing;
        self
    }

    pub fn set_debugger(&mut self, debugger: bool) -> &mut Self {
        self.inspector_config.debugger = debugger;
        self
    }

    pub fn set_trace_printer(&mut self, trace_printer: bool) -> &mut Self {
        self.inspector_config.trace_printer = trace_printer;
        self
    }

    pub fn set_gas_limit(&mut self, gas_limit: U256) -> &mut Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Calls the `setUp()` function on a contract.
    ///
    /// This will commit any state changes to the underlying database.
    ///
    /// Ayn changes made during the setup call to env's block environment are persistent, for
    /// example `vm.chainId()` will change the `block.chainId` for all subsequent test calls.
    pub fn setup(
        &mut self,
        from: Option<Address>,
        to: Address,
    ) -> Result<CallResult<()>, EvmError> {
        trace!(?from, ?to, "setting up contract");

        let from = from.unwrap_or(CALLER);
        self.backend_mut().set_test_contract(to).set_caller(from);
        let res = self.call_committing::<(), _, _>(from, to, "setUp()", (), 0.into(), None)?;

        // record any changes made to the block's environment during setup
        self.env.block = res.env.block.clone();
        // and also the chainid, which can be set manually
        self.env.cfg.chain_id = res.env.cfg.chain_id;

        match res.state_changeset.as_ref() {
            Some(changeset) => {
                let success = self
                    .ensure_success(to, res.reverted, changeset.clone(), false)
                    .map_err(|err| EvmError::Eyre(eyre::eyre!(err.to_string())))?;
                if success {
                    Ok(res)
                } else {
                    Err(EvmError::Execution(Box::new(ExecutionErr {
                        reverted: res.reverted,
                        reason: "execution error".to_owned(),
                        traces: res.traces,
                        gas_used: res.gas_used,
                        gas_refunded: res.gas_refunded,
                        stipend: res.stipend,
                        logs: res.logs,
                        debug: res.debug,
                        labels: res.labels,
                        state_changeset: None,
                        transactions: None,
                        script_wallets: res.script_wallets,
                    })))
                }
            }
            None => Ok(res),
        }
    }

    /// Performs a call to an account on the current state of the VM.
    ///
    /// The state after the call is persisted.
    pub fn call_committing<D: Detokenize, T: Tokenize, F: IntoFunction>(
        &mut self,
        from: Address,
        to: Address,
        func: F,
        args: T,
        value: U256,
        abi: Option<&Abi>,
    ) -> Result<CallResult<D>, EvmError> {
        let func = func.into();
        let calldata = Bytes::from(encode_function_data(&func, args)?.to_vec());
        let result = self.call_raw_committing(from, to, calldata, value)?;
        convert_call_result(abi, &func, result)
    }

    /// Performs a raw call to an account on the current state of the VM.
    ///
    /// The state after the call is persisted.
    pub fn call_raw_committing(
        &mut self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
    ) -> eyre::Result<RawCallResult> {
        let env = self.build_test_env(from, TransactTo::Call(h160_to_b160(to)), calldata, value);
        let mut result = self.call_raw_with_env(env)?;
        self.commit(&mut result);
        Ok(result)
    }

    /// Executes the test function call
    pub fn execute_test<D: Detokenize, T: Tokenize, F: IntoFunction>(
        &mut self,
        from: Address,
        test_contract: Address,
        func: F,
        args: T,
        value: U256,
        abi: Option<&Abi>,
    ) -> Result<CallResult<D>, EvmError> {
        let func = func.into();
        let calldata = Bytes::from(encode_function_data(&func, args)?.to_vec());

        // execute the call
        let env = self.build_test_env(from, TransactTo::Call(h160_to_b160(test_contract)), calldata, value);
        let call_result = self.call_raw_with_env(env)?;

        convert_call_result(abi, &func, call_result)
    }

    /// Performs a call to an account on the current state of the VM.
    ///
    /// The state after the call is not persisted.
    pub fn call<D: Detokenize, T: Tokenize, F: IntoFunction>(
        &self,
        from: Address,
        to: Address,
        func: F,
        args: T,
        value: U256,
        abi: Option<&Abi>,
    ) -> Result<CallResult<D>, EvmError> {
        let func = func.into();
        let calldata = Bytes::from(encode_function_data(&func, args)?.to_vec());
        let call_result = self.call_raw(from, to, calldata, value)?;
        convert_call_result(abi, &func, call_result)
    }

    /// Performs a raw call to an account on the current state of the VM.
    ///
    /// Any state modifications made by the call are not committed.
    pub fn call_raw(
        &self,
        from: Address,
        to: Address,
        calldata: Bytes,
        value: U256,
    ) -> eyre::Result<RawCallResult> {
        // execute the call
        let mut inspector = self.inspector_config.stack();
        // Build VM
        let mut env = self.build_test_env(from, TransactTo::Call(h160_to_b160(to)), calldata, value);
        let mut db = FuzzBackendWrapper::new(self.backend());
        let result = db.inspect_ref(&mut env, &mut inspector)?;

        convert_executed_result(env, inspector, result)
    }

    /// Execute the transaction configured in `env.tx` and commit the changes
    pub fn commit_tx_with_env(&mut self, env: Env) -> eyre::Result<RawCallResult> {
        let mut result = self.call_raw_with_env(env)?;
        self.commit(&mut result);
        Ok(result)
    }

    /// Execute the transaction configured in `env.tx`
    pub fn call_raw_with_env(&mut self, mut env: Env) -> eyre::Result<RawCallResult> {
        // execute the call
        let mut inspector = self.inspector_config.stack();
        let result = self.backend_mut().inspect_ref(&mut env, &mut inspector)?;
        convert_executed_result(env, inspector, result)
    }

    /// Commit the changeset to the database and adjust `self.inspector_config`
    /// values according to the executed call result
    fn commit(&mut self, result: &mut RawCallResult) {
        // persist changes to db
        if let Some(changes) = result.state_changeset.as_ref() {
            self.backend_mut().commit(changes.clone());
        }
        // Persist the changed block environment
        self.inspector_config.block = result.env.block.clone();
        // Persist cheatcode state
        let mut cheatcodes = result.cheatcodes.take();
        if let Some(cheats) = cheatcodes.as_mut() {
            // Clear broadcastable transactions
            cheats.broadcastable_transactions.clear();

            // corrected_nonce value is needed outside of this context (setUp), so we don't
            // reset it.
        }
        self.inspector_config.cheatcodes = cheatcodes;
    }

    /// Deploys a contract using the given `env` and commits the new state to the underlying
    /// database
    pub fn deploy_with_env(
        &mut self,
        env: Env,
        abi: Option<&Abi>,
    ) -> Result<DeployResult, EvmError> {
        debug_assert!(
            matches!(env.tx.transact_to, TransactTo::Create(_)),
            "Expect create transaction"
        );
        trace!(sender=?env.tx.caller, "deploying contract");

        let mut result = self.call_raw_with_env(env)?;
        self.commit(&mut result);

        let RawCallResult {
            exit_reason,
            out,
            gas_used,
            gas_refunded,
            logs,
            labels,
            traces,
            debug,
            script_wallets,
            env,
            ..
        } = result;

        let result = match out {
            Some(Output::Create(ref data, _)) => data.to_owned(),
            _ => Bytes::default(),
        };

        let address = match exit_reason {
            return_ok!() => {
                if let Some(Output::Create(_, Some(addr))) = out {
                    addr
                } else {
                    return Err(EvmError::Execution(Box::new(ExecutionErr {
                        reverted: true,
                        reason: "Deployment succeeded, but no address was returned. This is a bug, please report it".to_string(),
                        traces,
                        gas_used,
                        gas_refunded: 0,
                        stipend: 0,
                        logs,
                        debug,
                        labels,
                        state_changeset: None,
                        transactions: None,
                        script_wallets
                    })));
                }
            }
            _ => {
                let reason = decode::decode_revert(result.as_ref(), abi, Some(exit_reason))
                    .unwrap_or_else(|_| format!("{exit_reason:?}"));
                return Err(EvmError::Execution(Box::new(ExecutionErr {
                    reverted: true,
                    reason,
                    traces,
                    gas_used,
                    gas_refunded,
                    stipend: 0,
                    logs,
                    debug,
                    labels,
                    state_changeset: None,
                    transactions: None,
                    script_wallets,
                })))
            }
        };

        // also mark this library as persistent, this will ensure that the state of the library is
        // persistent across fork swaps in forking mode
        self.backend.add_persistent_account(b160_to_h160(address));

        trace!(address=?address, "deployed contract");

        Ok(DeployResult { address: b160_to_h160(address), gas_used, gas_refunded, logs, traces, debug, env })
    }

    /// Deploys a contract and commits the new state to the underlying database.
    ///
    /// Executes a CREATE transaction with the contract `code` and persistent database state
    /// modifications
    pub fn deploy(
        &mut self,
        from: Address,
        code: Bytes,
        value: U256,
        abi: Option<&Abi>,
    ) -> Result<DeployResult, EvmError> {
        let env = self.build_test_env(from, TransactTo::Create(CreateScheme::Create), code, value);
        self.deploy_with_env(env, abi)
    }

    /// Check if a call to a test contract was successful.
    ///
    /// This function checks both the VM status of the call, DSTest's `failed` status and the
    /// `globalFailed` flag which is stored in `failed` inside the `CHEATCODE_ADDRESS` contract.
    ///
    /// DSTest will not revert inside its `assertEq`-like functions which allows
    /// to test multiple assertions in 1 test function while also preserving logs.
    ///
    /// If an `assert` is violated, the contract's `failed` variable is set to true, and the
    /// `globalFailure` flag inside the `CHEATCODE_ADDRESS` is also set to true, this way, failing
    /// asserts from any contract are tracked as well.
    ///
    /// In order to check whether a test failed, we therefore need to evaluate the contract's
    /// `failed` variable and the `globalFailure` flag, which happens by calling
    /// `contract.failed()`.
    pub fn is_success(
        &self,
        address: Address,
        reverted: bool,
        state_changeset: StateChangeset,
        should_fail: bool,
    ) -> bool {
        self.ensure_success(address, reverted, state_changeset, should_fail).unwrap_or_default()
    }

    fn ensure_success(
        &self,
        address: Address,
        reverted: bool,
        state_changeset: StateChangeset,
        should_fail: bool,
    ) -> Result<bool, DatabaseError> {
        if self.backend().has_snapshot_failure() {
            // a failure occurred in a reverted snapshot, which is considered a failed test
            return Ok(should_fail)
        }

        // Construct a new VM with the state changeset
        let mut backend = self.backend().clone_empty();

        // we only clone the test contract and cheatcode accounts, that's all we need to evaluate
        // success
        for addr in [address, CHEATCODE_ADDRESS] {
            let acc = self.backend().basic(h160_to_b160(addr))?.unwrap_or_default();
            backend.insert_account_info(addr, acc);
        }

        // If this test failed any asserts, then this changeset will contain changes `false -> true`
        // for the contract's `failed` variable and the `globalFailure` flag in the state of the
        // cheatcode address which are both read when call `"failed()(bool)"` in the next step
        backend.commit(state_changeset);
        let executor =
            Executor::new(backend, self.env.clone(), self.inspector_config.clone(), self.gas_limit);

        let mut success = !reverted;
        if success {
            // Check if a DSTest assertion failed
            let call =
                executor.call::<bool, _, _>(CALLER, address, "failed()(bool)", (), 0.into(), None);

            if let Ok(CallResult { result: failed, .. }) = call {
                success = !failed;
            }
        }

        Ok(should_fail ^ success)
    }

    /// Creates the environment to use when executing a transaction in a test context
    ///
    /// If using a backend with cheatcodes, `tx.gas_price` and `block.number` will be overwritten by
    /// the cheatcode state inbetween calls.
    fn build_test_env(
        &self,
        caller: Address,
        transact_to: TransactTo,
        data: Bytes,
        value: U256,
    ) -> Env {
        Env {
            cfg: self.env.cfg.clone(),
            // We always set the gas price to 0 so we can execute the transaction regardless of
            // network conditions - the actual gas price is kept in `self.block` and is applied by
            // the cheatcode handler if it is enabled
            block: BlockEnv {
                basefee: rU256::from(0),
                gas_limit: self.gas_limit.into(),
                ..self.env.block.clone()
            },
            tx: TxEnv {
                caller: h160_to_b160(caller),
                transact_to,
                data,
                value,
                // As above, we set the gas price to 0.
                gas_price: rU256::from(0),
                gas_priority_fee: None,
                gas_limit: self.gas_limit.as_u64(),
                ..self.env.tx.clone()
            },
        }
    }
}

/// Represents the context after an execution error occurred.
#[derive(thiserror::Error, Debug)]
#[error("Execution reverted: {reason} (gas: {gas_used})")]
pub struct ExecutionErr {
    pub reverted: bool,
    pub reason: String,
    pub gas_used: u64,
    pub gas_refunded: u64,
    pub stipend: u64,
    pub logs: Vec<Log>,
    pub traces: Option<CallTraceArena>,
    pub debug: Option<DebugArena>,
    pub labels: BTreeMap<Address, String>,
    pub transactions: Option<BroadcastableTransactions>,
    pub state_changeset: Option<StateChangeset>,
    pub script_wallets: Vec<LocalWallet>,
}

#[derive(thiserror::Error, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum EvmError {
    /// Error which occurred during execution of a transaction
    #[error(transparent)]
    Execution(Box<ExecutionErr>),
    /// Error which occurred during ABI encoding/decoding
    #[error(transparent)]
    AbiError(#[from] ethers::contract::AbiError),
    /// Any other error.
    #[error(transparent)]
    Eyre(#[from] eyre::Error),
}

/// The result of a deployment.
#[derive(Debug)]
pub struct DeployResult {
    /// The address of the deployed contract
    pub address: Address,
    /// The gas cost of the deployment
    pub gas_used: u64,
    /// The refunded gas
    pub gas_refunded: u64,
    /// The logs emitted during the deployment
    pub logs: Vec<Log>,
    /// The traces of the deployment
    pub traces: Option<CallTraceArena>,
    /// The debug nodes of the call
    pub debug: Option<DebugArena>,
    /// The `revm::Env` after deployment
    pub env: Env,
}

/// The result of a call.
#[derive(Debug)]
pub struct CallResult<D: Detokenize> {
    /// Whether the call reverted or not
    pub reverted: bool,
    /// The decoded result of the call
    pub result: D,
    /// The gas used for the call
    pub gas_used: u64,
    /// The refunded gas for the call
    pub gas_refunded: u64,
    /// The initial gas stipend for the transaction
    pub stipend: u64,
    /// The logs emitted during the call
    pub logs: Vec<Log>,
    /// The labels assigned to addresses during the call
    pub labels: BTreeMap<Address, String>,
    /// The traces of the call
    pub traces: Option<CallTraceArena>,
    /// The coverage info collected during the call
    pub coverage: Option<HitMaps>,
    /// The debug nodes of the call
    pub debug: Option<DebugArena>,
    /// Scripted transactions generated from this call
    pub transactions: Option<BroadcastableTransactions>,
    /// The changeset of the state.
    ///
    /// This is only present if the changed state was not committed to the database (i.e. if you
    /// used `call` and `call_raw` not `call_committing` or `call_raw_committing`).
    pub state_changeset: Option<StateChangeset>,
    /// The wallets added during the call using the `rememberKey` cheatcode
    pub script_wallets: Vec<LocalWallet>,
    /// The `revm::Env` after the call
    pub env: Env,
}

/// The result of a raw call.
#[derive(Debug)]
pub struct RawCallResult {
    /// The status of the call
    pub exit_reason: Return,
    /// Whether the call reverted or not
    pub reverted: bool,
    /// The raw result of the call
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
    pub labels: BTreeMap<Address, String>,
    /// The traces of the call
    pub traces: Option<CallTraceArena>,
    /// The coverage info collected during the call
    pub coverage: Option<HitMaps>,
    /// The debug nodes of the call
    pub debug: Option<DebugArena>,
    /// Scripted transactions generated from this call
    pub transactions: Option<BroadcastableTransactions>,
    /// The changeset of the state.
    ///
    /// This is only present if the changed state was not committed to the database (i.e. if you
    /// used `call` and `call_raw` not `call_committing` or `call_raw_committing`).
    pub state_changeset: Option<StateChangeset>,
    /// The wallets added during the call using the `rememberKey` cheatcode
    pub script_wallets: Vec<LocalWallet>,
    /// The `revm::Env` after the call
    pub env: Env,
    /// The cheatcode states after execution
    pub cheatcodes: Option<Cheatcodes>,
    /// The raw output of the execution
    pub out: Option<Output>,
    /// The chisel state
    pub chisel_state: Option<(revm::Stack, revm::Memory, revm::Return)>,
}

impl Default for RawCallResult {
    fn default() -> Self {
        Self {
            exit_reason: Return::Continue,
            reverted: false,
            result: Bytes::new(),
            gas_used: 0,
            gas_refunded: 0,
            stipend: 0,
            logs: Vec::new(),
            labels: BTreeMap::new(),
            traces: None,
            coverage: None,
            debug: None,
            transactions: None,
            state_changeset: None,
            script_wallets: Vec::new(),
            env: Default::default(),
            cheatcodes: Default::default(),
            out: None, // TODO: missing in revm
            chisel_state: None,
        }
    }
}

/// Calculates the initial gas stipend for a transaction
fn calc_stipend(calldata: &[u8], spec: SpecId) -> u64 {
    let non_zero_data_cost = if SpecId::enabled(spec, SpecId::ISTANBUL) { 16 } else { 68 };
    calldata.iter().fold(21000, |sum, byte| sum + if *byte == 0 { 4 } else { non_zero_data_cost })
}

/// Converts the data aggregated in the `inspector` and `call` to a `RawCallResult`
fn convert_executed_result(
    env: Env,
    inspector: InspectorStack,
    result: ResultAndState,
) -> eyre::Result<RawCallResult> {
    let ResultAndState { result: exec_result, state: state_changeset } = result;
    // need:
    // exit_reason
    // gas_refunded
    // gas_used
    // output
    let (exit_reason, gas_refunded, gas_used, out) = match exec_result {
        ExecutionResult::Success { reason, gas_used, gas_refunded, logs, output } => {
            (eval_to_instruction_result(reason), gas_refunded, gas_used, Some(output))
        },
        ExecutionResult::Revert { gas_used, .. } => {
            (InstructionResult::Revert, 0 as u64, gas_used, None)
        },
        ExecutionResult::Halt { reason, gas_used } => {
            (halt_to_instruction_result(reason), 0 as u64, gas_used, None)
        },
    };

    let stipend = calc_stipend(&env.tx.data, env.cfg.spec_id);

    let result = match out {
        Some(Output::Call(ref data)) => data.to_owned(),
        _ => Bytes::default(),
    };

    let InspectorData {
        logs,
        labels,
        traces,
        coverage,
        debug,
        cheatcodes,
        script_wallets,
        chisel_state,
    } = inspector.collect_inspector_states();

    let transactions = match cheatcodes.as_ref() {
        Some(cheats) if !cheats.broadcastable_transactions.is_empty() => {
            Some(cheats.broadcastable_transactions.clone())
        }
        _ => None,
    };

    Ok(RawCallResult {
        exit_reason,
        reverted: !matches!(exit_reason, return_ok!()),
        result,
        gas_used,
        gas_refunded,
        stipend,
        logs: logs.to_vec(),
        labels,
        traces,
        coverage,
        debug,
        transactions,
        state_changeset: Some(state_changeset),
        script_wallets,
        env,
        cheatcodes,
        out,
        chisel_state,
    })
}

fn convert_call_result<D: Detokenize>(
    abi: Option<&Contract>,
    func: &Function,
    call_result: RawCallResult,
) -> Result<CallResult<D>, EvmError> {
    let RawCallResult {
        result,
        exit_reason: status,
        reverted,
        gas_used,
        gas_refunded,
        stipend,
        logs,
        labels,
        traces,
        coverage,
        debug,
        transactions,
        state_changeset,
        script_wallets,
        env,
        ..
    } = call_result;

    match status {
        return_ok!() => {
            let result = decode_function_data(func, result, false)?;
            Ok(CallResult {
                reverted,
                result,
                gas_used,
                gas_refunded,
                stipend,
                logs,
                labels,
                traces,
                coverage,
                debug,
                transactions,
                state_changeset,
                script_wallets,
                env,
            })
        }
        _ => {
            let reason = decode::decode_revert(result.as_ref(), abi, Some(status))
                .unwrap_or_else(|_| format!("{status:?}"));
            Err(EvmError::Execution(Box::new(ExecutionErr {
                reverted,
                reason,
                gas_used,
                gas_refunded,
                stipend,
                logs,
                traces,
                debug,
                labels,
                transactions,
                state_changeset,
                script_wallets,
            })))
        }
    }
}
