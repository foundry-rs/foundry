mod build;
use build::BuildOutput;

mod runner;
use runner::Runner;

mod broadcast;
use ui::{TUIExitReason, Tui, Ui};

mod cmd;

mod executor;

mod receipts;

use crate::{cmd::forge::build::BuildArgs, opts::MultiWallet};
use clap::{Parser, ValueHint};
use ethers::{
    abi::{Abi, Function},
    prelude::{artifacts::ContractBytecodeSome, ArtifactId, Bytes, Project},
    types::{transaction::eip2718::TypedTransaction, Address, Log, TransactionRequest, U256},
};
use forge::{
    debug::DebugArena,
    decode::decode_console_logs,
    executor::opts::EvmOpts,
    trace::{
        identifier::{EtherscanIdentifier, LocalTraceIdentifier},
        CallTraceArena, CallTraceDecoder, CallTraceDecoderBuilder, TraceKind,
    },
};

use foundry_common::evm::EvmArgs;
use foundry_config::Config;
use foundry_utils::{encode_args, format_token, IntoFunction};

use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    time::Duration,
};

use yansi::Paint;

// Loads project's figment and merges the build cli arguments into it
foundry_config::impl_figment_convert!(ScriptArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser)]
pub struct ScriptArgs {
    /// The contract you want to run. Either the file path or contract name.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[clap(value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub path: String,

    /// Arguments to pass to the script function.
    #[clap(value_name = "ARGS")]
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[clap(long, visible_alias = "tc", value_name = "CONTRACT_NAME")]
    pub target_contract: Option<String>,

    /// The signature of the function you want to call in the contract, or raw calldata.
    #[clap(long, short, default_value = "run()", value_name = "SIGNATURE")]
    pub sig: String,

    #[clap(
        long,
        help = "Use legacy transactions instead of EIP1559 ones. this is auto-enabled for common networks without EIP1559."
    )]
    pub legacy: bool,

    #[clap(long, help = "Broadcasts the transactions.")]
    pub broadcast: bool,

    #[clap(flatten, next_help_heading = "BUILD OPTIONS")]
    pub opts: BuildArgs,

    #[clap(flatten)]
    pub wallets: MultiWallet,

    #[clap(flatten, next_help_heading = "EVM OPTIONS")]
    pub evm_opts: EvmArgs,

    /// Resumes submitting transactions that failed or timed-out previously.
    ///
    /// It DOES NOT simulate the script again and it expects nonces to have remained the same.
    ///
    /// Example: If transaction N has a nonce of 22, then the account should have a nonce of 22,
    /// otherwise it fails.
    #[clap(long)]
    pub resume: bool,

    #[clap(long, help = "Takes precedence over broadcast")]
    pub debug: bool,

    #[clap(
        long,
        help = "Makes sure a transaction is sent, only after its previous one has been confirmed and succeeded."
    )]
    pub slow: bool,
}

pub struct ScriptResult {
    pub success: bool,
    pub logs: Vec<Log>,
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    pub debug: Option<Vec<DebugArena>>,
    pub gas: u64,
    pub labeled_addresses: BTreeMap<Address, String>,
    pub transactions: Option<VecDeque<TypedTransaction>>,
    pub returned: bytes::Bytes,
}

impl ScriptArgs {
    pub fn decode_traces(
        &self,
        script_config: &ScriptConfig,
        result: &mut ScriptResult,
        known_contracts: &BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
    ) -> eyre::Result<CallTraceDecoder> {
        let etherscan_identifier = EtherscanIdentifier::new(
            script_config.evm_opts.get_remote_chain_id(),
            script_config.config.etherscan_api_key.clone(),
            Config::foundry_etherscan_cache_dir(script_config.evm_opts.get_chain_id()),
            Duration::from_secs(24 * 60 * 60),
        );

        let local_identifier = LocalTraceIdentifier::new(known_contracts);
        let mut decoder =
            CallTraceDecoderBuilder::new().with_labels(result.labeled_addresses.clone()).build();

        for (_, trace) in &mut result.traces {
            decoder.identify(trace, &etherscan_identifier);
            decoder.identify(trace, &local_identifier);
        }
        Ok(decoder)
    }

    pub fn show_traces(
        &self,
        script_config: &ScriptConfig,
        decoder: &CallTraceDecoder,
        result: &mut ScriptResult,
    ) -> eyre::Result<()> {
        let verbosity = script_config.evm_opts.verbosity;
        let func = script_config.called_function.as_ref().expect("There should be a function.");

        if verbosity >= 3 {
            if result.traces.is_empty() {
                eyre::bail!("Unexpected error: No traces despite verbosity level. Please report this as a bug: https://github.com/foundry-rs/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
            }

            if !result.success && verbosity == 3 || verbosity > 3 {
                println!("Traces:");
                for (kind, trace) in &mut result.traces {
                    let should_include = match kind {
                        TraceKind::Setup => (verbosity >= 5) || (verbosity == 4 && !result.success),
                        TraceKind::Execution => verbosity > 3 || !result.success,
                        _ => false,
                    };

                    if should_include {
                        decoder.decode(trace);
                        println!("{trace}");
                    }
                }
                println!();
            }
        }

        if result.success {
            println!("{}", Paint::green("Script ran successfully."));
        } else {
            println!("{}", Paint::red("Script failed."));
        }

        println!("Gas used: {}", result.gas);

        if !result.returned.is_empty() {
            println!("\n== Return ==");
            match func.decode_output(&result.returned) {
                Ok(decoded) => {
                    for (index, (token, output)) in decoded.iter().zip(&func.outputs).enumerate() {
                        let internal_type = output.internal_type.as_deref().unwrap_or("unknown");

                        let label = if !output.name.is_empty() {
                            output.name.to_string()
                        } else {
                            index.to_string()
                        };
                        println!("{}: {} {}", label.trim_end(), internal_type, format_token(token));
                    }
                }
                Err(_) => {
                    println!("{:x?}", (&result.returned));
                }
            }
        }

        let console_logs = decode_console_logs(&result.logs);
        if !console_logs.is_empty() {
            println!("\n== Logs ==");
            for log in console_logs {
                println!("  {}", log);
            }
        }

        if !result.success {
            eyre::bail!("{}", Paint::red("Script failed."));
        }

        Ok(())
    }

    /// It finds the deployer from the running script and uses it to predeploy libraries.
    ///
    /// If there are multiple candidate addresses, it skips everything and lets `--sender` deploy
    /// them instead.
    fn maybe_new_sender(
        &self,
        evm_opts: &EvmOpts,
        transactions: Option<&VecDeque<TypedTransaction>>,
        predeploy_libraries: &[Bytes],
    ) -> eyre::Result<Option<Address>> {
        let mut new_sender = None;

        if let Some(txs) = transactions {
            if !predeploy_libraries.is_empty() {
                for tx in txs.iter() {
                    match tx {
                        TypedTransaction::Legacy(tx) => {
                            if tx.to.is_none() {
                                let sender = tx.from.expect("no sender");
                                if let Some(ns) = new_sender {
                                    if sender != ns {
                                        println!("You have more than one deployer who could predeploy libraries. Using `--sender` instead.");
                                        return Ok(None)
                                    }
                                } else if sender != evm_opts.sender {
                                    new_sender = Some(sender);
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        Ok(new_sender)
    }

    /// Helper for building the transactions for any libraries that need to be deployed ahead of
    /// linking
    fn create_deploy_transactions(
        &self,
        from: Address,
        nonce: U256,
        data: &[Bytes],
    ) -> VecDeque<TypedTransaction> {
        data.iter()
            .enumerate()
            .map(|(i, bytes)| {
                TypedTransaction::Legacy(TransactionRequest {
                    from: Some(from),
                    data: Some(bytes.clone()),
                    nonce: Some(nonce + i),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn run_debugger(
        &self,
        decoder: &CallTraceDecoder,
        sources: BTreeMap<u32, String>,
        result: ScriptResult,
        project: Project,
        highlevel_known_contracts: BTreeMap<ArtifactId, ContractBytecodeSome>,
    ) -> eyre::Result<()> {
        let source_code: BTreeMap<u32, String> = sources
            .iter()
            .map(|(id, path)| {
                let resolved = project
                    .paths
                    .resolve_library_import(&PathBuf::from(path))
                    .unwrap_or_else(|| PathBuf::from(path));
                (
                    *id,
                    std::fs::read_to_string(resolved).expect(&*format!(
                        "Something went wrong reading the source file: {:?}",
                        path
                    )),
                )
            })
            .collect();

        let calls: Vec<DebugArena> = result.debug.expect("we should have collected debug info");
        let flattened = calls.last().expect("we should have collected debug info").flatten(0);
        let tui = Tui::new(
            flattened,
            0,
            decoder.contracts.clone(),
            highlevel_known_contracts
                .into_iter()
                .map(|(id, artifact)| (id.name, artifact))
                .collect(),
            source_code,
        )?;
        match tui.start().expect("Failed to start tui") {
            TUIExitReason::CharExit => Ok(()),
        }
    }

    pub fn get_method_and_calldata(&self, abi: &Abi) -> eyre::Result<(Function, Bytes)> {
        let (func, data) = match self.sig.strip_prefix("0x") {
            Some(calldata) => (
                abi.functions()
                    .find(|&func| {
                        func.short_signature().to_vec() == hex::decode(calldata).unwrap()[..4]
                    })
                    .expect("Function selector not found in the ABI"),
                hex::decode(calldata).unwrap().into(),
            ),
            _ => {
                let func = IntoFunction::into(self.sig.clone());
                (
                    abi.functions()
                        .find(|&abi_func| abi_func.short_signature() == func.short_signature())
                        .expect("Function signature not found in the ABI"),
                    encode_args(&func, &self.args)?.into(),
                )
            }
        };
        Ok((func.clone(), data))
    }
}

pub struct ScriptConfig {
    pub config: foundry_config::Config,
    pub evm_opts: EvmOpts,
    pub sender_nonce: U256,
    pub called_function: Option<Function>,
}
