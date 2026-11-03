use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes, Log, U256, hex, map::HashMap};
use clap::{Parser, ValueHint};
use eyre::{Result, WrapErr, bail};
use foundry_common::{sh_println, shell};
use foundry_config::{Config, SymbolicStorageLayout};
use foundry_evm::{
    backend::Backend,
    constants::{CALLER, TEST_CONTRACT_ADDRESS},
    core::evm::EthEvmNetwork,
    executors::{Executor, ExecutorBuilder},
    opts::EvmOpts,
    revm::{bytecode::Bytecode, database::DatabaseRef},
};
use foundry_evm_symbolic::{
    ExecutorView, SymbolicEquivalenceInput, SymbolicEquivalenceResult, SymbolicExecutor,
    SymbolicStats,
};
use serde_json::json;
use std::{collections::BTreeSet, path::PathBuf};

/// Default raw calldata lengths: empty, a selector, and a selector with one to four words.
const DEFAULT_CALLDATA_LENGTHS: [usize; 6] = [0, 4, 36, 68, 100, 132];

/// CLI arguments for `forge equivalence`.
#[derive(Clone, Debug, Parser)]
pub struct EquivalenceArgs {
    /// Runtime bytecode of the first contract, as hex.
    #[arg(long, value_name = "HEX", required_unless_present = "code_a_file")]
    pub code_a: Option<String>,

    /// File holding the runtime bytecode of the first contract, as hex.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath, conflicts_with = "code_a")]
    pub code_a_file: Option<PathBuf>,

    /// Runtime bytecode of the second contract, as hex.
    #[arg(long, value_name = "HEX", required_unless_present = "code_b_file")]
    pub code_b: Option<String>,

    /// File holding the runtime bytecode of the second contract, as hex.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath, conflicts_with = "code_b")]
    pub code_b_file: Option<PathBuf>,

    /// Only explore ABI-encoded calls of this function, such as `transfer(address,uint256)`.
    #[arg(long, value_name = "SIGNATURE")]
    pub sig: Option<String>,

    /// Lengths in bytes of the fully symbolic calldata to explore when `--sig` is not given.
    #[arg(
        long,
        value_name = "LENGTHS",
        value_delimiter = ',',
        conflicts_with = "sig",
        default_values_t = DEFAULT_CALLDATA_LENGTHS
    )]
    pub calldata_lengths: Vec<usize>,

    /// Start from arbitrary symbolic storage instead of empty storage.
    #[arg(long)]
    pub symbolic_storage: bool,

    /// Solver to use, such as z3, cvc5, yices or bitwuzla. Defaults to `symbolic.solver`.
    #[arg(long, value_name = "SOLVER")]
    pub solver: Option<String>,

    /// Timeout for the whole check, in seconds. Defaults to `symbolic.timeout`.
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u32>,

    /// Call value, in wei, sent with the call to both contracts.
    #[arg(long, value_name = "WEI", default_value_t = U256::ZERO)]
    pub value: U256,

    /// Starting balance of the contract, in wei, before the call value is added.
    #[arg(long, value_name = "WEI", default_value_t = U256::ZERO)]
    pub balance: U256,
}

/// One concrete run of either contract on the counterexample.
struct ConcreteRun {
    success: bool,
    output: Bytes,
    storage: HashMap<U256, U256>,
    logs: Vec<Log>,
    balances: HashMap<Address, U256>,
}

impl EquivalenceArgs {
    pub async fn run(self) -> Result<()> {
        let code_a = read_code(self.code_a.as_deref(), self.code_a_file.as_ref(), "--code-a")?;
        let code_b = read_code(self.code_b.as_deref(), self.code_b_file.as_ref(), "--code-b")?;
        let function = self
            .sig
            .as_deref()
            .map(|sig| Function::parse(sig).wrap_err_with(|| format!("invalid --sig `{sig}`")))
            .transpose()?;

        let config = Config::load()?;
        let mut symbolic = config.symbolic.clone();
        if let Some(solver) = self.solver {
            symbolic.solver = solver;
        }
        if let Some(timeout) = self.timeout {
            symbolic.timeout = Some(timeout);
        }
        symbolic.storage_layout = if self.symbolic_storage {
            SymbolicStorageLayout::Generic
        } else {
            SymbolicStorageLayout::ZeroInit
        };

        let evm_opts = Config::figment().extract::<EvmOpts>()?;
        let (evm_env, tx_env, _) = evm_opts.env_with_fork_context().await?;
        let backend = Backend::<EthEvmNetwork>::spawn(None)?;
        let mut executor = ExecutorBuilder::<EthEvmNetwork>::new()
            .inspectors(|stack| stack.logs(false))
            .gas_limit(evm_opts.gas_limit())
            .build(evm_env, tx_env, backend, evm_opts.networks);
        executor.set_code(TEST_CONTRACT_ADDRESS, Bytecode::new_raw(code_a.clone()))?;
        executor.set_balance(TEST_CONTRACT_ADDRESS, self.balance)?;
        if executor.get_balance(CALLER)? < self.value {
            executor.set_balance(CALLER, self.value)?;
        }

        let result = SymbolicExecutor::new(symbolic).run_equivalence(SymbolicEquivalenceInput {
            executor: ExecutorView::new(&executor),
            address: TEST_CONTRACT_ADDRESS,
            code_a: code_a.clone(),
            code_b: code_b.clone(),
            sender: CALLER,
            value: self.value,
            function: function.as_ref(),
            calldata_lengths: self.calldata_lengths.clone(),
        });

        match result {
            SymbolicEquivalenceResult::Equivalent { stats } => {
                if shell::is_json() {
                    sh_println!("{}", json!({ "status": "equivalent", "stats": stats }))?;
                } else {
                    sh_println!("Equivalent: no explored input distinguishes the two contracts.")?;
                    print_stats(&stats)?;
                }
                Ok(())
            }
            SymbolicEquivalenceResult::Incomplete { kind, reason, .. } => {
                bail!("equivalence check incomplete ({kind:?}): {reason}")
            }
            SymbolicEquivalenceResult::Counterexample { calldata, args, storage, stats } => {
                for assignment in &storage {
                    executor.set_storage_slot(
                        assignment.address,
                        assignment.slot,
                        assignment.value,
                    )?;
                }
                let run_a = replay(&mut executor, &code_a, &calldata, self.value)?;
                let run_b = replay(&mut executor, &code_b, &calldata, self.value)?;
                let slots = run_a
                    .storage
                    .keys()
                    .chain(run_b.storage.keys())
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|slot| {
                        Ok((
                            slot,
                            final_slot(&executor, &run_a, slot)?,
                            final_slot(&executor, &run_b, slot)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .filter(|(_, a, b)| a != b)
                    .collect::<Vec<_>>();
                let balances = run_a
                    .balances
                    .keys()
                    .chain(run_b.balances.keys())
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|account| {
                        Ok((
                            account,
                            final_balance(&executor, &run_a, account)?,
                            final_balance(&executor, &run_b, account)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .filter(|(_, a, b)| a != b)
                    .collect::<Vec<_>>();
                let both_succeeded = run_a.success && run_b.success;
                let logs_differ = both_succeeded && run_a.logs != run_b.logs;
                let differs = run_a.success != run_b.success
                    || run_a.output != run_b.output
                    || (both_succeeded && (!slots.is_empty() || !balances.is_empty()))
                    || logs_differ;
                if !differs {
                    bail!(
                        "the solver's counterexample {calldata} did not replay concretely; \
                         equivalence is not established"
                    );
                }

                if shell::is_json() {
                    let side = |run: &ConcreteRun| {
                        json!({
                            "success": run.success,
                            "output": run.output,
                            "logs": run.logs.iter().map(log_json).collect::<Vec<_>>(),
                        })
                    };
                    let balances = balances
                        .iter()
                        .map(|(account, a, b)| {
                            json!({ "address": account, "code_a": a, "code_b": b })
                        })
                        .collect::<Vec<_>>();
                    let storage = slots
                        .iter()
                        .map(|(slot, a, b)| json!({ "slot": slot, "code_a": a, "code_b": b }))
                        .collect::<Vec<_>>();
                    sh_println!(
                        "{}",
                        json!({
                            "status": "not_equivalent",
                            "calldata": calldata,
                            "args": args.iter().map(format_value).collect::<Vec<_>>(),
                            "code_a": side(&run_a),
                            "code_b": side(&run_b),
                            "storage": storage,
                            "balances": balances,
                            "stats": stats,
                        })
                    )?;
                } else {
                    sh_println!("Not equivalent: the contracts differ on calldata {calldata}")?;
                    if function.is_some() {
                        let args = args.iter().map(format_value).collect::<Vec<_>>().join(", ");
                        sh_println!("  args:   ({args})")?;
                    }
                    sh_println!("  code-a: {}", describe(&run_a))?;
                    sh_println!("  code-b: {}", describe(&run_b))?;
                    for (slot, a, b) in &slots {
                        sh_println!("  storage slot {slot:#x}: code-a {a:#x}, code-b {b:#x}")?;
                    }
                    for (account, a, b) in &balances {
                        sh_println!("  balance of {account}: code-a {a}, code-b {b}")?;
                    }
                    if logs_differ {
                        sh_println!("  code-a logs:")?;
                        print_logs(&run_a.logs)?;
                        sh_println!("  code-b logs:")?;
                        print_logs(&run_b.logs)?;
                    }
                    print_stats(&stats)?;
                }
                bail!("contracts are not equivalent")
            }
        }
    }
}

/// Reads hex bytecode from a flag or a file.
fn read_code(hex_code: Option<&str>, file: Option<&PathBuf>, flag: &str) -> Result<Bytes> {
    let text = match (hex_code, file) {
        (Some(code), _) => code.to_string(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read {flag}-file {}", path.display()))?,
        (None, None) => bail!("{flag} or {flag}-file is required"),
    };
    let text = text.split_whitespace().collect::<String>();
    hex::decode(&text).map(Bytes::from).wrap_err_with(|| format!("{flag} is not valid hex"))
}

/// Runs `code` at the shared address on `calldata` without committing state.
///
/// `Executor::call_raw` runs with a zero gas price and base fee, so balances only move by value
/// transfers and self-destructs, as in the symbolic model.
fn replay(
    executor: &mut Executor<EthEvmNetwork>,
    code: &Bytes,
    calldata: &Bytes,
    value: U256,
) -> Result<ConcreteRun> {
    executor.set_code(TEST_CONTRACT_ADDRESS, Bytecode::new_raw(code.clone()))?;
    let result = executor.call_raw(CALLER, TEST_CONTRACT_ADDRESS, calldata.clone(), value)?;
    let balances = result
        .state_changeset
        .iter()
        .filter(|(_, account)| account.is_touched())
        .map(|(address, account)| (*address, account.info.balance))
        .collect();
    let storage = result
        .state_changeset
        .get(&TEST_CONTRACT_ADDRESS)
        .map(|account| {
            account
                .storage
                .iter()
                .filter(|(_, slot)| slot.is_changed())
                .map(|(key, slot)| (*key, slot.present_value()))
                .collect()
        })
        .unwrap_or_default();
    Ok(ConcreteRun {
        success: !result.reverted,
        output: result.result,
        storage,
        logs: result.logs,
        balances,
    })
}

/// Returns a slot's value after `run`, falling back to the shared pre-state.
fn final_slot(executor: &Executor<EthEvmNetwork>, run: &ConcreteRun, slot: U256) -> Result<U256> {
    if !run.success {
        return Ok(executor.backend().storage_ref(TEST_CONTRACT_ADDRESS, slot)?);
    }
    match run.storage.get(&slot) {
        Some(value) => Ok(*value),
        None => Ok(executor.backend().storage_ref(TEST_CONTRACT_ADDRESS, slot)?),
    }
}

/// Returns an account's balance after `run`, falling back to the shared pre-state.
fn final_balance(
    executor: &Executor<EthEvmNetwork>,
    run: &ConcreteRun,
    account: Address,
) -> Result<U256> {
    match run.balances.get(&account) {
        Some(balance) if run.success => Ok(*balance),
        _ => Ok(executor.get_balance(account)?),
    }
}

fn log_json(log: &Log) -> serde_json::Value {
    json!({ "address": log.address, "topics": log.topics(), "data": log.data.data })
}

fn print_logs(logs: &[Log]) -> Result<()> {
    if logs.is_empty() {
        sh_println!("    (none)")?;
    }
    for log in logs {
        let topics = log.topics().iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
        sh_println!("    {} topics [{topics}] data {}", log.address, log.data.data)?;
    }
    Ok(())
}

fn describe(run: &ConcreteRun) -> String {
    let verb = if run.success { "returned" } else { "reverted with" };
    if run.output.is_empty() { format!("{verb} no data") } else { format!("{verb} {}", run.output) }
}

fn format_value(value: &DynSolValue) -> String {
    foundry_common::fmt::format_token(value)
}

fn print_stats(stats: &SymbolicStats) -> Result<()> {
    sh_println!("Explored {} paths with {} solver queries.", stats.paths, stats.solver_queries)?;
    Ok(())
}
