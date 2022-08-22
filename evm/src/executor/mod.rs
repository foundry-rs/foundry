use self::inspector::{InspectorData, InspectorStackConfig};
use crate::{debug::DebugArena, decode, trace::CallTraceArena, CALLER};
pub use abi::{
    format_hardhat_call, patch_hardhat_console_selector, HardhatConsoleCalls, CHEATCODE_ADDRESS,
    CONSOLE_ABI, HARDHAT_CONSOLE_ABI, HARDHAT_CONSOLE_ADDRESS,
};
use backend::FuzzBackendWrapper;
use bytes::Bytes;
use ethers::{
    abi::{Abi, Contract, Detokenize, Function, Tokenize},
    prelude::{decode_function_data, encode_function_data, Address, U256},
    types::{transaction::eip2718::TypedTransaction, Log},
};
use foundry_utils::IntoFunction;
use hashbrown::HashMap;
use revm::{
    db::DatabaseCommit, return_ok, Account, BlockEnv, Bytecode, CreateScheme, ExecutionResult,
    Return, TransactOut, TransactTo, TxEnv, EVM,
};
/// Reexport commonly used revm types
pub use revm::{db::DatabaseRef, Env, SpecId};
use std::collections::{BTreeMap, VecDeque};
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
        backend::DatabaseExt,
        inspector::{InspectorStack, DEFAULT_CREATE2_DEPLOYER},
    },
};
pub use builder::ExecutorBuilder;

/// A mapping of addresses to their changed state.
pub type StateChangeset = HashMap<Address, Account>;

/// A type that can execute calls
///
/// The executor can be configured with various `revm::Inspector`s, like `Cheatcodes`.
///
/// There are two ways of executing calls:
///   - `committing`: any state changes made during the call are recorded and are persisting
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
            revm::AccountInfo {
                code: Some(Bytecode::new_raw(vec![0u8].into()).to_checked()),
                ..Default::default()
            },
        );

        Executor { backend, env, inspector_config, gas_limit }
    }

    /// Returns a reference to the Env
    pub fn env(&mut self) -> &Env {
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
        let create2_deployer_account = self.backend_mut().basic(DEFAULT_CREATE2_DEPLOYER);

        if create2_deployer_account.code.is_none() ||
            create2_deployer_account.code.as_ref().unwrap().is_empty()
        {
            let creator = "0x3fAB184622Dc19b6109349B94811493BF2a45362".parse().unwrap();

            // Probably 0, but just in case.
            let initial_balance = self.get_balance(creator);

            self.set_balance(creator, U256::MAX);
            self.deploy(
                creator,
                hex::decode("604580600e600039806000f350fe7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3").expect("Could not decode create2 deployer init_code").into(),
                U256::zero(),
                None
            )?;
            self.set_balance(creator, initial_balance);
        }
        Ok(())
    }

    /// Set the balance of an account.
    pub fn set_balance(&mut self, address: Address, amount: U256) -> &mut Self {
        trace!(?address, ?amount, "setting account balance");
        let mut account = self.backend_mut().basic(address);
        account.balance = amount;

        self.backend_mut().insert_account_info(address, account);
        self
    }

    /// Gets the balance of an account
    pub fn get_balance(&self, address: Address) -> U256 {
        self.backend().basic(address).balance
    }

    /// Set the nonce of an account.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) -> &mut Self {
        let mut account = self.backend_mut().basic(address);
        account.nonce = nonce;

        self.backend_mut().insert_account_info(address, account);
        self
    }

    pub fn set_tracing(&mut self, tracing: bool) -> &mut Self {
        self.inspector_config.tracing = tracing;
        self
    }

    pub fn set_debugger(&mut self, debugger: bool) -> &mut Self {
        self.inspector_config.debugger = debugger;
        self
    }

    pub fn set_gas_limit(&mut self, gas_limit: U256) -> &mut Self {
        self.gas_limit = gas_limit;
        self
    }

    /// Calls the `setUp()` function on a contract.
    ///
    /// This will commit any state changes to the underlying database
    pub fn setup(
        &mut self,
        from: Option<Address>,
        to: Address,
    ) -> Result<CallResult<()>, EvmError> {
        let from = from.unwrap_or(CALLER);
        self.backend_mut().set_test_contract(to).set_caller(from);
        self.call_committing::<(), _, _>(from, to, "setUp()", (), 0.into(), None)
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
        let RawCallResult {
            result,
            exit_reason,
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
        } = self.call_raw_committing(from, to, calldata, value)?;
        match exit_reason {
            return_ok!() => {
                let result = decode_function_data(&func, result, false)?;
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
                })
            }
            _ => {
                let reason = decode::decode_revert(result.as_ref(), abi, Some(exit_reason))
                    .unwrap_or_else(|_| format!("{:?}", exit_reason));
                Err(EvmError::Execution {
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
                })
            }
        }
    }

    /// Execute the transaction configured in `env.tx` and commit the state to the database
    pub fn commit_tx_with_env(&mut self, env: Env) -> eyre::Result<RawCallResult> {
        let stipend = calc_stipend(&env.tx.data, env.cfg.spec_id);

        // Build VM
        let mut evm = EVM::new();
        evm.env = env;
        let mut inspector = self.inspector_config.stack();
        evm.database(self.backend_mut());

        // Run the call
        let ExecutionResult { exit_reason, out, gas_used, gas_refunded, .. } =
            evm.inspect_commit(&mut inspector);
        let result = match out {
            TransactOut::Call(data) => data,
            _ => Bytes::default(),
        };

        let InspectorData { logs, labels, traces, coverage, debug, mut cheatcodes } =
            inspector.collect_inspector_states();

        // Persist the changed block environment
        self.inspector_config.block = evm.env.block.clone();

        let transactions = if let Some(ref mut cheatcodes) = cheatcodes {
            if !cheatcodes.broadcastable_transactions.is_empty() {
                let transactions = Some(cheatcodes.broadcastable_transactions.clone());

                // Clear broadcast state from cheatcode state
                cheatcodes.broadcastable_transactions.clear();
                cheatcodes.corrected_nonce = false;

                transactions
            } else {
                None
            }
        } else {
            None
        };

        // Persist cheatcode state
        self.inspector_config.cheatcodes = cheatcodes;

        Ok(RawCallResult {
            exit_reason,
            reverted: !matches!(exit_reason, return_ok!()),
            result,
            gas_used,
            gas_refunded,
            stipend,
            logs,
            labels,
            coverage,
            traces,
            debug,
            transactions,
            state_changeset: None,
        })
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
        let env = self.build_test_env(from, TransactTo::Call(to), calldata, value);
        self.commit_tx_with_env(env)
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
        let mut inspector = self.inspector_config.stack();
        let stipend = calc_stipend(&calldata, self.env.cfg.spec_id);
        let env = self.build_test_env(from, TransactTo::Call(test_contract), calldata, value);
        let (ExecutionResult { exit_reason, out, gas_used, gas_refunded, logs }, state_changeset) =
            self.backend_mut().inspect_ref(env, &mut inspector);

        // if there are multiple forks we need to merge them
        let logs = self.backend.merged_logs(logs);

        let executed_call = ExecutedCall {
            exit_reason,
            out,
            gas_used,
            gas_refunded,
            state_changeset,
            logs,
            stipend,
        };
        let call_result = convert_executed_call(inspector, executed_call)?;

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
        let stipend = calc_stipend(&calldata, self.env.cfg.spec_id);
        // Build VM
        let env = self.build_test_env(from, TransactTo::Call(to), calldata, value);
        let mut db = FuzzBackendWrapper::new(self.backend());
        let (ExecutionResult { exit_reason, out, gas_used, gas_refunded, logs }, state_changeset) =
            db.inspect_ref(env, &mut inspector);
        let logs = db.backend.merged_logs(logs);

        let executed_call = ExecutedCall {
            exit_reason,
            out,
            gas_used,
            gas_refunded,
            state_changeset,
            logs,
            stipend,
        };
        convert_executed_call(inspector, executed_call)
    }

    /// Deploys a contract using the given `env` and commits the new state to the underlying
    /// database
    pub fn deploy_with_env(
        &mut self,
        env: Env,
        abi: Option<&Abi>,
    ) -> Result<DeployResult, EvmError> {
        trace!(sender=?env.tx.caller, "deploying contract");

        let mut inspector = self.inspector_config.stack();
        let (ExecutionResult { exit_reason, out, gas_used, gas_refunded, .. }, env) = {
            let mut evm = EVM::new();
            evm.env = env;
            evm.database(self.backend_mut());
            let res = evm.inspect_commit(&mut inspector);
            (res, evm.env)
        };

        let InspectorData { logs, labels, traces, debug, cheatcodes, .. } =
            inspector.collect_inspector_states();

        let result = match out {
            TransactOut::Create(ref data, _) => data.to_owned(),
            _ => Bytes::default(),
        };

        let address = match exit_reason {
            return_ok!() => {
                if let TransactOut::Create(_, Some(addr)) = out {
                    addr
                } else {
                    return Err(EvmError::Execution {
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
                        transactions: None
                    });
                }
            }
            _ => {
                let reason = decode::decode_revert(result.as_ref(), abi, Some(exit_reason))
                    .unwrap_or_else(|_| format!("{:?}", exit_reason));
                return Err(EvmError::Execution {
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
                })
            }
        };

        // also mark this library as persistent, this will ensure that the state of the library is
        // persistent across fork swaps in forking mode
        self.backend.add_persistent_account(address);

        // Persist the changed block environment
        self.inspector_config.block = env.block;

        // Persist cheatcode state
        self.inspector_config.cheatcodes = cheatcodes;

        trace!(address=?address, "deployed contract");

        Ok(DeployResult { address, gas_used, gas_refunded, logs, traces, debug })
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
    /// This function checks both the VM status of the call and DSTest's `failed`.
    ///
    /// DSTest will not revert inside its `assertEq`-like functions which allows
    /// to test multiple assertions in 1 test function while also preserving logs.
    ///
    /// Instead, it sets `failed` to `true` which we must check.
    // TODO(mattsse): check if safe to replace with `Backend::is_failed()`
    pub fn is_success(
        &self,
        address: Address,
        reverted: bool,
        state_changeset: StateChangeset,
        should_fail: bool,
    ) -> bool {
        // Construct a new VM with the state changeset
        let mut backend = self.backend().clone_empty();
        backend.insert_account_info(address, self.backend().basic(address));
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

        should_fail ^ success
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
                basefee: 0.into(),
                gas_limit: self.gas_limit,
                ..self.env.block.clone()
            },
            tx: TxEnv {
                caller,
                transact_to,
                data,
                value,
                // As above, we set the gas price to 0.
                gas_price: 0.into(),
                gas_priority_fee: None,
                gas_limit: self.gas_limit.as_u64(),
                ..self.env.tx.clone()
            },
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum EvmError {
    /// Error which occurred during execution of a transaction
    #[error("Execution reverted: {reason} (gas: {gas_used})")]
    Execution {
        reverted: bool,
        reason: String,
        gas_used: u64,
        gas_refunded: u64,
        stipend: u64,
        logs: Vec<Log>,
        traces: Option<CallTraceArena>,
        debug: Option<DebugArena>,
        labels: BTreeMap<Address, String>,
        transactions: Option<VecDeque<TypedTransaction>>,
        state_changeset: Option<StateChangeset>,
    },
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
    pub transactions: Option<VecDeque<TypedTransaction>>,
    /// The changeset of the state.
    ///
    /// This is only present if the changed state was not committed to the database (i.e. if you
    /// used `call` and `call_raw` not `call_committing` or `call_raw_committing`).
    pub state_changeset: Option<StateChangeset>,
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
    pub transactions: Option<VecDeque<TypedTransaction>>,
    /// The changeset of the state.
    ///
    /// This is only present if the changed state was not committed to the database (i.e. if you
    /// used `call` and `call_raw` not `call_committing` or `call_raw_committing`).
    pub state_changeset: Option<StateChangeset>,
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
        }
    }
}

/// Helper type to bundle all call related items
struct ExecutedCall {
    exit_reason: Return,
    out: TransactOut,
    gas_used: u64,
    gas_refunded: u64,
    state_changeset: HashMap<Address, Account>,
    #[allow(unused)]
    logs: Vec<revm::Log>,
    stipend: u64,
}

/// Calculates the initial gas stipend for a transaction
fn calc_stipend(calldata: &[u8], spec: SpecId) -> u64 {
    let non_zero_data_cost = if SpecId::enabled(spec, SpecId::ISTANBUL) { 16 } else { 68 };
    calldata.iter().fold(21000, |sum, byte| sum + if *byte == 0 { 4 } else { non_zero_data_cost })
}

/// Converts the data aggregated in the `inspector` and `call` to a `RawCallResult`
fn convert_executed_call(
    inspector: InspectorStack,
    call: ExecutedCall,
) -> eyre::Result<RawCallResult> {
    let ExecutedCall { exit_reason, out, gas_used, gas_refunded, state_changeset, stipend, .. } =
        call;

    let result = match out {
        TransactOut::Call(data) => data,
        _ => Bytes::default(),
    };

    let InspectorData { logs, labels, traces, coverage, debug, cheatcodes } =
        inspector.collect_inspector_states();

    let transactions = if let Some(cheats) = cheatcodes {
        if !cheats.broadcastable_transactions.is_empty() {
            Some(cheats.broadcastable_transactions)
        } else {
            None
        }
    } else {
        None
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
            })
        }
        _ => {
            let reason = decode::decode_revert(result.as_ref(), abi, Some(status))
                .unwrap_or_else(|_| format!("{:?}", status));
            Err(EvmError::Execution {
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
            })
        }
    }
}
