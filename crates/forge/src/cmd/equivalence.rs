use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Bytes, Log, TxKind, U256, hex, map::HashMap};
use clap::{Parser, ValueHint};
use eyre::{Result, WrapErr, bail};
use foundry_common::{sh_println, shell};
use foundry_config::{Config, SymbolicConfig, SymbolicStorageLayout};
use foundry_evm::{
    backend::Backend,
    constants::{CALLER, TEST_CONTRACT_ADDRESS},
    core::{FoundryBlock, FoundryTransaction, evm::EthEvmNetwork},
    executors::{Executor, ExecutorBuilder},
    opts::EvmOpts,
    revm::{bytecode::Bytecode, database::DatabaseRef, interpreter::InstructionResult},
};
use foundry_evm_symbolic::{
    ExecutorView, SymbolicEquivalenceEnv, SymbolicEquivalenceInput, SymbolicEquivalenceResult,
    SymbolicExecutor, SymbolicStats, SymbolicStorageAssignment,
};
use serde_json::json;
use std::{collections::BTreeSet, path::PathBuf};

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

    /// Explore raw calldata of exactly these lengths, in bytes, instead of a symbolic length.
    #[arg(long, value_name = "LENGTHS", value_delimiter = ',', conflicts_with = "sig")]
    pub calldata_lengths: Vec<usize>,

    /// Largest symbolic calldata, in bytes.
    #[arg(long, value_name = "BYTES", default_value_t = 1024)]
    pub max_calldata: usize,

    /// Largest return data of a call into unknown code, in bytes.
    #[arg(long, value_name = "BYTES", default_value_t = 256)]
    pub max_returndata: usize,

    /// Start from empty storage instead of arbitrary symbolic storage.
    #[arg(long)]
    pub empty_storage: bool,

    /// Fix the call value, in wei, instead of leaving it symbolic.
    #[arg(long, value_name = "WEI")]
    pub value: Option<U256>,

    /// Assume calls into unknown code never call back into the contracts, so state-changing
    /// calls into it are abstracted instead of stopping the check.
    #[arg(long)]
    pub promise_no_reent: bool,

    /// Compare the contracts modulo gas, like hevm: leave out inputs on which either contract
    /// runs out of gas instead of reporting them as a difference.
    #[arg(long)]
    pub ignore_out_of_gas: bool,

    /// Solver to use, such as z3, cvc5, yices or bitwuzla. Defaults to bitwuzla when it is
    /// installed and `symbolic.solver` is left at its default, and to `symbolic.solver` otherwise.
    #[arg(long, value_name = "SOLVER")]
    pub solver: Option<String>,

    /// Timeout for the whole check, in seconds. Defaults to `symbolic.timeout`.
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u32>,
}

/// One concrete run of either contract on the counterexample.
struct ConcreteRun {
    success: bool,
    output: Bytes,
    /// Storage slots the run changed, of every account.
    storage: HashMap<(Address, U256), U256>,
    logs: Vec<Log>,
    balances: HashMap<Address, U256>,
    /// Nonce and code hash of every account the run touched.
    accounts: HashMap<Address, (u64, B256)>,
    /// Whether the run ran out of gas.
    out_of_gas: bool,
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
        } else if symbolic.solver == SymbolicConfig::default().solver
            && symbolic.solver_command.as_deref().is_none_or(str::is_empty)
            && symbolic.solver_portfolio.is_empty()
            && bitwuzla_available()
        {
            // Like hevm, prefer bitwuzla, which answers equivalence queries several times faster
            // than z3.
            symbolic.solver = "bitwuzla".to_string();
        }
        if let Some(timeout) = self.timeout {
            symbolic.timeout = Some(timeout);
        }
        symbolic.storage_layout = if self.empty_storage {
            SymbolicStorageLayout::ZeroInit
        } else {
            SymbolicStorageLayout::Generic
        };
        symbolic.max_calldata_bytes = symbolic.max_calldata_bytes.max(self.max_calldata as u32);

        let evm_opts = Config::figment().extract::<EvmOpts>()?;
        let (evm_env, tx_env, _) = evm_opts.env_with_fork_context().await?;
        let backend = Backend::<EthEvmNetwork>::spawn(None)?;
        let mut executor = ExecutorBuilder::<EthEvmNetwork>::new()
            .inspectors(|stack| stack.logs(false))
            .gas_limit(evm_opts.gas_limit())
            .build(evm_env, tx_env, backend, evm_opts.networks);
        executor.set_code(TEST_CONTRACT_ADDRESS, Bytecode::new_raw(code_a.clone()))?;

        let result = SymbolicExecutor::new(symbolic).run_equivalence(SymbolicEquivalenceInput {
            executor: ExecutorView::new(&executor),
            address: TEST_CONTRACT_ADDRESS,
            code_a: code_a.clone(),
            code_b: code_b.clone(),
            sender: CALLER,
            value: self.value,
            function: function.as_ref(),
            calldata_lengths: self.calldata_lengths.clone(),
            max_calldata: self.max_calldata,
            max_returndata: self.max_returndata,
            promise_no_reent: self.promise_no_reent,
            ignore_out_of_gas: self.ignore_out_of_gas,
        });

        let (calldata, args, storage, env, unknown_calls, reads_gas, stats) = match result {
            SymbolicEquivalenceResult::Equivalent { stats } => {
                if shell::is_json() {
                    sh_println!("{}", json!({ "status": "equivalent", "stats": stats }))?;
                } else {
                    sh_println!("Equivalent: no explored input distinguishes the two contracts.")?;
                    print_stats(&stats)?;
                }
                return Ok(());
            }
            SymbolicEquivalenceResult::Incomplete { kind, reason, .. } => {
                bail!("equivalence check incomplete ({kind:?}): {reason}")
            }
            SymbolicEquivalenceResult::Counterexample {
                calldata,
                args,
                storage,
                env,
                unknown_calls,
                reads_gas,
                stats,
            } => (calldata, args, storage, env, unknown_calls, reads_gas, stats),
        };

        let accounts = AccountMap { address: env.address, caller: env.caller };
        prepare_replay(&mut executor, &env, &storage, &accounts)?;
        let run_a = replay(&mut executor, &env, &code_a, &calldata)?;
        let run_b = replay(&mut executor, &env, &code_b, &calldata)?;
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
        let accounts_differ = run_a
            .accounts
            .keys()
            .chain(run_b.accounts.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|account| {
                Ok((
                    account,
                    final_account(&executor, &run_a, account)?,
                    final_account(&executor, &run_b, account)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            // The contracts' own code differs by construction.
            // The contracts' own code differs by construction, and the sender's nonce is the
            // transaction's.
            .filter(|(account, (nonce_a, code_a), (nonce_b, code_b))| {
                (nonce_a != nonce_b && *account != env.caller)
                    || (code_a != code_b && *account != env.address)
            })
            .collect::<Vec<_>>();
        let both_succeeded = run_a.success && run_b.success;
        let logs_differ = both_succeeded && run_a.logs != run_b.logs;
        let replayed = run_a.success != run_b.success
            || run_a.output != run_b.output
            || (both_succeeded
                && (!slots.is_empty() || !balances.is_empty() || !accounts_differ.is_empty()))
            || logs_differ;
        // A model that answers calls into unknown code, reads `GAS` or picks a separate origin
        // cannot always be replayed by plain execution; like hevm, it is still reported.
        let unconfirmed = (!replayed).then(|| {
            let mut reasons = Vec::new();
            if unknown_calls > 0 {
                reasons.push(format!("answers {unknown_calls} calls into unknown code"));
            }
            if reads_gas {
                reasons.push("depends on GAS".to_string());
            }
            if env.origin != env.caller {
                reasons.push("needs tx.origin to differ from the sender".to_string());
            }
            reasons
        });
        if let Some(reasons) = &unconfirmed
            && reasons.is_empty()
        {
            bail!(
                "equivalence check incomplete (Error): the solver's counterexample {calldata} did \
                 not replay concretely"
            );
        }
        let unconfirmed = unconfirmed.map(|reasons| reasons.join(", "));

        if shell::is_json() {
            let side = |run: &ConcreteRun| {
                json!({
                    "success": run.success,
                    "output": run.output,
                    "logs": run.logs.iter().map(log_json).collect::<Vec<_>>(),
                })
            };
            let storage = slots
                .iter()
                .map(|((address, slot), a, b)| {
                    json!({ "address": address, "slot": slot, "code_a": a, "code_b": b })
                })
                .collect::<Vec<_>>();
            let balances = balances
                .iter()
                .map(|(account, a, b)| json!({ "address": account, "code_a": a, "code_b": b }))
                .collect::<Vec<_>>();
            sh_println!(
                "{}",
                json!({
                    "status": "not_equivalent",
                    "replayed": replayed,
                    "out_of_gas": run_a.out_of_gas != run_b.out_of_gas,
                    "unconfirmed": unconfirmed,
                    "calldata": calldata,
                    "args": args.iter().map(format_value).collect::<Vec<_>>(),
                    "env": env_json(&env),
                    "code_a": side(&run_a),
                    "code_b": side(&run_b),
                    "storage": storage,
                    "balances": balances,
                    "accounts": accounts_differ
                        .iter()
                        .map(|(account, (nonce_a, code_a), (nonce_b, code_b))| json!({
                            "address": account,
                            "code_a": { "nonce": nonce_a, "code_hash": code_a },
                            "code_b": { "nonce": nonce_b, "code_hash": code_b },
                        }))
                        .collect::<Vec<_>>(),
                    "stats": stats,
                })
            )?;
        } else {
            sh_println!("Not equivalent: the contracts differ on calldata {calldata}")?;
            if function.is_some() {
                let args = args.iter().map(format_value).collect::<Vec<_>>().join(", ");
                sh_println!("  args:   ({args})")?;
            }
            sh_println!(
                "  sender {}, value {}, block {}, timestamp {}",
                env.caller,
                env.value,
                env.number,
                env.timestamp
            )?;
            if let Some(reasons) = &unconfirmed {
                sh_println!(
                    "  The solver's counterexample {reasons}, so it was not replayed concretely."
                )?;
            } else {
                if run_a.out_of_gas != run_b.out_of_gas {
                    sh_println!(
                        "  Only one contract runs out of gas here; pass --ignore-out-of-gas to \
                         compare them modulo gas, as hevm does."
                    )?;
                }
                sh_println!("  code-a: {}", describe(&run_a))?;
                sh_println!("  code-b: {}", describe(&run_b))?;
                for ((address, slot), a, b) in &slots {
                    if *address == env.address {
                        sh_println!("  storage slot {slot:#x}: code-a {a:#x}, code-b {b:#x}")?;
                    } else {
                        sh_println!(
                            "  storage slot {slot:#x} of {address}: code-a {a:#x}, code-b {b:#x}"
                        )?;
                    }
                }
                for (account, a, b) in &balances {
                    sh_println!("  balance of {account}: code-a {a}, code-b {b}")?;
                }
                for (account, (nonce_a, code_a), (nonce_b, code_b)) in
                    accounts_differ.iter().filter(|_| both_succeeded)
                {
                    sh_println!(
                        "  account {account}: code-a nonce {nonce_a} code {code_a}, code-b nonce \
                         {nonce_b} code {code_b}"
                    )?;
                }
                if logs_differ {
                    sh_println!("  code-a logs:")?;
                    print_logs(&run_a.logs)?;
                    sh_println!("  code-b logs:")?;
                    print_logs(&run_b.logs)?;
                }
            }
            print_stats(&stats)?;
        }
        bail!("contracts are not equivalent")
    }
}

/// Maps the accounts that stand for the symbolic address and sender to the model's choices.
struct AccountMap {
    address: Address,
    caller: Address,
}

impl AccountMap {
    fn map(&self, account: Address) -> Address {
        if account == TEST_CONTRACT_ADDRESS {
            self.address
        } else if account == CALLER {
            self.caller
        } else {
            account
        }
    }
}

/// Returns whether a bitwuzla binary runs.
fn bitwuzla_available() -> bool {
    std::process::Command::new("bitwuzla")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
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

/// Puts the counterexample's pre-state and block environment into the concrete executor.
fn prepare_replay(
    executor: &mut Executor<EthEvmNetwork>,
    env: &SymbolicEquivalenceEnv,
    storage: &[SymbolicStorageAssignment],
    accounts: &AccountMap,
) -> Result<()> {
    for (account, balance) in &env.balances {
        executor.set_balance(accounts.map(*account), *balance)?;
    }
    for assignment in storage {
        executor.set_storage_slot(
            accounts.map(assignment.address),
            assignment.slot,
            assignment.value,
        )?;
    }
    let evm_env = executor.evm_env_mut();
    evm_env.cfg_env.chain_id = env.chain_id.saturating_to();
    // Gas is not priced in the symbolic model, so the replay charges no fees either.
    evm_env.cfg_env.disable_fee_charge = true;
    evm_env.cfg_env.disable_base_fee = true;
    let block = &mut evm_env.block_env;
    block.set_number(env.number);
    block.set_timestamp(env.timestamp);
    block.set_beneficiary(env.coinbase);
    block.set_difficulty(env.prevrandao);
    block.set_prevrandao(Some(B256::from(env.prevrandao)));
    block.set_basefee(env.basefee.saturating_to());
    Ok(())
}

/// Runs `code` at the counterexample's address on `calldata` without committing state.
///
/// Fees are not charged, so balances only move by value transfers and self-destructs, as in the
/// symbolic model.
fn replay(
    executor: &mut Executor<EthEvmNetwork>,
    env: &SymbolicEquivalenceEnv,
    code: &Bytes,
    calldata: &Bytes,
) -> Result<ConcreteRun> {
    executor.set_code(env.address, Bytecode::new_raw(code.clone()))?;
    let (mut evm_env, mut tx_env) = executor.prepare_call_env(
        env.caller,
        TxKind::Call(env.address),
        calldata.clone(),
        env.value,
    );
    evm_env.block_env.set_basefee(env.basefee.saturating_to());
    tx_env.set_gas_price(env.gas_price.saturating_to());
    let result = executor.call_with_env(evm_env, tx_env)?;
    let storage = result
        .state_changeset
        .iter()
        .flat_map(|(address, account)| {
            account
                .storage
                .iter()
                .filter(|(_, slot)| slot.is_changed())
                .map(|(key, slot)| ((*address, *key), slot.present_value()))
        })
        .collect();
    let balances = result
        .state_changeset
        .iter()
        .filter(|(_, account)| account.is_touched())
        .map(|(address, account)| (*address, account.info.balance))
        .collect();
    let accounts = result
        .state_changeset
        .iter()
        .filter(|(_, account)| account.is_touched())
        .map(|(address, account)| (*address, (account.info.nonce, account.info.code_hash)))
        .collect();
    Ok(ConcreteRun {
        success: !result.reverted,
        output: result.result,
        storage,
        logs: result.logs,
        balances,
        accounts,
        out_of_gas: result.exit_reason.is_some_and(|reason| {
            matches!(
                reason,
                InstructionResult::OutOfGas
                    | InstructionResult::MemoryOOG
                    | InstructionResult::MemoryLimitOOG
                    | InstructionResult::PrecompileOOG
                    | InstructionResult::InvalidOperandOOG
            )
        }),
    })
}

/// Returns a slot's value after `run`, falling back to the shared pre-state.
fn final_slot(
    executor: &Executor<EthEvmNetwork>,
    run: &ConcreteRun,
    (address, slot): (Address, U256),
) -> Result<U256> {
    match run.storage.get(&(address, slot)) {
        Some(value) if run.success => Ok(*value),
        _ => Ok(executor.backend().storage_ref(address, slot)?),
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

/// Returns an account's nonce and code hash after `run`, falling back to the shared pre-state.
fn final_account(
    executor: &Executor<EthEvmNetwork>,
    run: &ConcreteRun,
    account: Address,
) -> Result<(u64, B256)> {
    match run.accounts.get(&account) {
        Some(state) if run.success => Ok(*state),
        _ => Ok(executor
            .backend()
            .basic_ref(account)?
            .map(|info| (info.nonce, info.code_hash))
            .unwrap_or((0, alloy_primitives::KECCAK256_EMPTY))),
    }
}

fn env_json(env: &SymbolicEquivalenceEnv) -> serde_json::Value {
    json!({
        "address": env.address,
        "caller": env.caller,
        "origin": env.origin,
        "value": env.value,
        "timestamp": env.timestamp,
        "number": env.number,
        "prevrandao": env.prevrandao,
        "gas_limit": env.gas_limit,
        "basefee": env.basefee,
        "chain_id": env.chain_id,
        "coinbase": env.coinbase,
        "gas_price": env.gas_price,
    })
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
    if run.out_of_gas {
        return "ran out of gas".to_string();
    }
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
