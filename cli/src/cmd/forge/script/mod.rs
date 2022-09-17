//! script command
use crate::{
    cmd::forge::build::BuildArgs,
    opts::MultiWallet,
    utils::{get_contract_name, parse_ether_value},
};
use cast::{decode, executor::inspector::DEFAULT_CREATE2_DEPLOYER};
use clap::{Parser, ValueHint};
use dialoguer::Confirm;
use ethers::{
    abi::{Abi, Function, HumanReadableParser},
    prelude::{
        artifacts::{ContractBytecodeSome, Libraries},
        ArtifactId, Bytes, Project,
    },
    signers::LocalWallet,
    types::{
        transaction::eip2718::TypedTransaction, Address, Log, NameOrAddress, TransactionRequest,
        U256,
    },
};
use eyre::{ContextCompat, WrapErr};
use forge::{
    debug::DebugArena,
    decode::decode_console_logs,
    executor::{opts::EvmOpts, Backend},
    trace::{
        identifier::{EtherscanIdentifier, LocalTraceIdentifier, SignaturesIdentifier},
        CallTraceArena, CallTraceDecoder, CallTraceDecoderBuilder, RawOrDecodedCall,
        RawOrDecodedReturnData, TraceKind,
    },
    CallKind,
};
use foundry_common::{evm::EvmArgs, ContractsByArtifact, CONTRACT_MAX_SIZE, SELECTOR_LEN};
use foundry_config::{figment, Config};
use foundry_utils::{encode_args, format_token};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
};
use yansi::Paint;

mod build;
use build::{filter_sources_and_artifacts, BuildOutput};
use foundry_config::figment::{
    value::{Dict, Map},
    Metadata, Profile, Provider,
};

mod runner;
use runner::ScriptRunner;

mod broadcast;
use ui::{TUIExitReason, Tui, Ui};

mod cmd;
mod executor;
mod receipts;
mod sequence;
mod transaction;
mod verify;

use crate::cmd::retry::RetryArgs;
pub use transaction::TransactionWithMetadata;

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(ScriptArgs, opts, evm_opts);

#[derive(Debug, Clone, Parser, Default)]
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
    #[clap(
        long,
        short,
        default_value = "run()",
        value_name = "SIGNATURE",
        value_parser = foundry_common::clap_helpers::strip_0x_prefix
    )]
    pub sig: String,

    #[clap(
        long,
        help = "Use legacy transactions instead of EIP1559 ones. this is auto-enabled for common networks without EIP1559."
    )]
    pub legacy: bool,

    #[clap(long, help = "Broadcasts the transactions.")]
    pub broadcast: bool,

    #[clap(long, help = "Skips on-chain simulation")]
    pub skip_simulation: bool,

    #[clap(
        long,
        short,
        default_value = "130",
        value_name = "GAS_ESTIMATE_MULTIPLIER",
        help = "Relative percentage to multiply gas estimates by"
    )]
    pub gas_estimate_multiplier: u64,

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

    #[clap(long, help = "Open the script in the debugger. Takes precedence over broadcast.")]
    pub debug: bool,

    #[clap(
        long,
        help = "Makes sure a transaction is sent, only after its previous one has been confirmed and succeeded."
    )]
    pub slow: bool,

    #[clap(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    pub etherscan_api_key: Option<String>,

    #[clap(
        long,
        help = "If it finds a matching broadcast log, it tries to verify every contract found in the receipts."
    )]
    pub verify: bool,

    #[clap(flatten)]
    pub verifier: super::verify::VerifierArgs,

    #[clap(long, help = "Output results in JSON format.")]
    pub json: bool,

    #[clap(
        long,
        help = "Gas price for legacy transactions, or max fee per gas for EIP1559 transactions.",
        env = "ETH_GAS_PRICE",
        parse(try_from_str = parse_ether_value),
        value_name = "PRICE"
    )]
    pub with_gas_price: Option<U256>,

    #[clap(flatten, help = "Allows to use retry arguments for contract verification")]
    pub retry: RetryArgs,
}

// === impl ScriptArgs ===

impl ScriptArgs {
    pub fn decode_traces(
        &self,
        script_config: &ScriptConfig,
        result: &mut ScriptResult,
        known_contracts: &ContractsByArtifact,
    ) -> eyre::Result<CallTraceDecoder> {
        let mut etherscan_identifier = EtherscanIdentifier::new(
            &script_config.config,
            script_config.evm_opts.get_remote_chain_id(),
        )?;

        let mut local_identifier = LocalTraceIdentifier::new(known_contracts);
        let mut decoder =
            CallTraceDecoderBuilder::new().with_labels(result.labeled_addresses.clone()).build();

        decoder.add_signature_identifier(SignaturesIdentifier::new(Config::foundry_cache_dir())?);

        for (_, trace) in &mut result.traces {
            decoder.identify(trace, &mut local_identifier);
            decoder.identify(trace, &mut etherscan_identifier);
        }
        Ok(decoder)
    }

    pub fn get_returns(
        &self,
        script_config: &ScriptConfig,
        returned: &bytes::Bytes,
    ) -> eyre::Result<HashMap<String, NestedValue>> {
        let func = script_config.called_function.as_ref().expect("There should be a function.");
        let mut returns = HashMap::new();

        match func.decode_output(returned) {
            Ok(decoded) => {
                for (index, (token, output)) in decoded.iter().zip(&func.outputs).enumerate() {
                    let internal_type = output.internal_type.as_deref().unwrap_or("unknown");

                    let label = if !output.name.is_empty() {
                        output.name.to_string()
                    } else {
                        index.to_string()
                    };

                    returns.insert(
                        label,
                        NestedValue {
                            internal_type: internal_type.to_string(),
                            value: format_token(token),
                        },
                    );
                }
            }
            Err(_) => {
                println!("{:x?}", (&returned));
            }
        }

        Ok(returns)
    }

    pub async fn show_traces(
        &self,
        script_config: &ScriptConfig,
        decoder: &CallTraceDecoder,
        result: &mut ScriptResult,
    ) -> eyre::Result<()> {
        let verbosity = script_config.evm_opts.verbosity;
        let func = script_config.called_function.as_ref().expect("There should be a function.");

        if !result.success || verbosity > 3 {
            if result.traces.is_empty() {
                eyre::bail!("Unexpected error: No traces despite verbosity level. Please report this as a bug: https://github.com/foundry-rs/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
            }

            println!("Traces:");
            for (kind, trace) in &mut result.traces {
                let should_include = match kind {
                    TraceKind::Setup => verbosity >= 5,
                    TraceKind::Execution => verbosity > 3,
                    _ => false,
                } || !result.success;

                if should_include {
                    decoder.decode(trace).await;
                    println!("{trace}");
                }
            }
            println!();
        }

        if result.success {
            println!("{}", Paint::green("Script ran successfully."));
        }

        if script_config.evm_opts.fork_url.is_none() {
            println!("Gas used: {}", result.gas_used);
        }

        if result.success && !result.returned.is_empty() {
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
            let revert_msg = decode::decode_revert(&result.returned[..], None, None)
                .map(|err| format!("{}\n", err))
                .unwrap_or_else(|_| "Script failed.\n".to_string());

            eyre::bail!("{}", Paint::red(revert_msg));
        }

        Ok(())
    }

    pub fn show_json(
        &self,
        script_config: &ScriptConfig,
        result: &ScriptResult,
    ) -> eyre::Result<()> {
        let returns = self.get_returns(script_config, &result.returned)?;

        let console_logs = decode_console_logs(&result.logs);
        let output = JsonResult { logs: console_logs, gas_used: result.gas_used, returns };
        let j = serde_json::to_string(&output)?;
        println!("{}", j);

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
        let (sources, artifacts) = filter_sources_and_artifacts(
            &self.path,
            sources,
            highlevel_known_contracts.clone(),
            project,
        )?;
        let flattened = result
            .debug
            .and_then(|arena| arena.last().map(|arena| arena.flatten(0)))
            .expect("We should have collected debug information");
        let identified_contracts = decoder
            .contracts
            .iter()
            .map(|(addr, identifier)| (*addr, get_contract_name(identifier).to_string()))
            .collect();

        let tui = Tui::new(
            flattened,
            0,
            identified_contracts,
            artifacts,
            highlevel_known_contracts
                .into_iter()
                .map(|(id, _)| (id.name, sources.clone()))
                .collect(),
        )?;
        match tui.start().expect("Failed to start tui") {
            TUIExitReason::CharExit => Ok(()),
        }
    }

    /// Returns the Function and calldata based on the signature
    ///
    /// If the `sig` is a valid human-readable function we find the corresponding function in the
    /// `abi` If the `sig` is valid hex, we assume it's calldata and try to find the
    /// corresponding function by matching the selector, first 4 bytes in the calldata.
    ///
    /// Note: We assume that the `sig` is already stripped of its prefix, See [`ScriptArgs`]
    pub fn get_method_and_calldata(&self, abi: &Abi) -> eyre::Result<(Function, Bytes)> {
        let (func, data) = if let Ok(func) = HumanReadableParser::parse_function(&self.sig) {
            (
                abi.functions()
                    .find(|&abi_func| abi_func.short_signature() == func.short_signature())
                    .wrap_err(format!(
                        "Function `{}` is not implemented in your script.",
                        self.sig
                    ))?,
                encode_args(&func, &self.args)?.into(),
            )
        } else {
            let decoded = hex::decode(&self.sig).wrap_err("Invalid hex calldata")?;
            let selector = &decoded[..SELECTOR_LEN];
            (
                abi.functions().find(|&func| selector == &func.short_signature()[..]).ok_or_else(
                    || {
                        eyre::eyre!(
                            "Function selector `{}` not found in the ABI",
                            hex::encode(selector)
                        )
                    },
                )?,
                decoded.into(),
            )
        };

        Ok((func.clone(), data))
    }

    /// Checks if the transaction is a deployment with a size above the `CONTRACT_MAX_SIZE`.
    ///
    /// If `self.broadcast` is enabled, it asks confirmation of the user. Otherwise, it just warns
    /// the user.
    fn check_contract_sizes(
        &self,
        result: &ScriptResult,
        known_contracts: &BTreeMap<ArtifactId, ContractBytecodeSome>,
    ) -> eyre::Result<()> {
        // (name, &init, &deployed)[]
        let mut bytecodes: Vec<(String, &[u8], &[u8])> = vec![];

        // From artifacts
        known_contracts.iter().for_each(|(artifact, bytecode)| {
            const MSG: &str = "Code should have been linked before.";
            let init_code = bytecode.bytecode.object.as_bytes().expect(MSG);
            // Ignore abstract contracts
            if let Some(ref deployed_code) = bytecode.deployed_bytecode.bytecode {
                let deployed_code = deployed_code.object.as_bytes().expect(MSG);
                bytecodes.push((artifact.name.clone(), init_code, deployed_code));
            }
        });

        // From traces
        let create_nodes = result.traces.iter().flat_map(|(_, traces)| {
            traces
                .arena
                .iter()
                .filter(|node| matches!(node.kind(), CallKind::Create | CallKind::Create2))
        });
        let mut unknown_c = 0usize;
        for node in create_nodes {
            // Calldata == init code
            if let RawOrDecodedCall::Raw(ref init_code) = node.trace.data {
                // Output is the runtime code
                if let RawOrDecodedReturnData::Raw(ref deployed_code) = node.trace.output {
                    // Only push if it was not present already
                    if !bytecodes.iter().any(|(_, b, _)| b == init_code) {
                        bytecodes.push((format!("Unknown{}", unknown_c), init_code, deployed_code));
                        unknown_c += 1;
                    }
                    continue
                }
            }
            // Both should be raw and not decoded since it's just bytecode
            eyre::bail!("Create node returned decoded data: {:?}", node);
        }

        let mut prompt_user = false;
        for (data, to) in result.transactions.iter().flat_map(|txes| {
            txes.iter().filter_map(|tx| {
                tx.data().filter(|data| data.len() > CONTRACT_MAX_SIZE).map(|data| (data, tx.to()))
            })
        }) {
            let mut offset = 0;

            // Find if it's a CREATE or CREATE2. Otherwise, skip transaction.
            if let Some(NameOrAddress::Address(to)) = to {
                if *to == DEFAULT_CREATE2_DEPLOYER {
                    // Size of the salt prefix.
                    offset = 32;
                }
            } else if to.is_some() {
                continue
            }

            // Find artifact with a deployment code same as the data.
            if let Some((name, _, deployed_code)) =
                bytecodes.iter().find(|(_, init_code, _)| *init_code == &data[offset..])
            {
                let deployment_size = deployed_code.len();

                if deployment_size > CONTRACT_MAX_SIZE {
                    prompt_user = self.broadcast;
                    println!(
                        "{}",
                        Paint::red(format!(
                            "`{}` is above the EIP-170 contract size limit ({} > {}).",
                            name, deployment_size, CONTRACT_MAX_SIZE
                        ))
                    );
                }
            }
        }

        if prompt_user &&
            !Confirm::new().with_prompt("Do you wish to continue?".to_string()).interact()?
        {
            eyre::bail!("User canceled the script.");
        }

        Ok(())
    }
}

impl Provider for ScriptArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Script Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::default();
        if let Some(ref etherscan_api_key) = self.etherscan_api_key {
            dict.insert(
                "etherscan_api_key".to_string(),
                figment::value::Value::from(etherscan_api_key.to_string()),
            );
        }
        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

pub struct ScriptResult {
    pub success: bool,
    pub logs: Vec<Log>,
    pub traces: Vec<(TraceKind, CallTraceArena)>,
    pub debug: Option<Vec<DebugArena>>,
    pub gas_used: u64,
    pub labeled_addresses: BTreeMap<Address, String>,
    pub transactions: Option<VecDeque<TypedTransaction>>,
    pub returned: bytes::Bytes,
    pub address: Option<Address>,
    pub script_wallets: Vec<LocalWallet>,
}

#[derive(Serialize, Deserialize)]
pub struct JsonResult {
    pub logs: Vec<String>,
    pub gas_used: u64,
    pub returns: HashMap<String, NestedValue>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct NestedValue {
    pub internal_type: String,
    pub value: String,
}

pub struct ScriptConfig {
    pub config: Config,
    pub evm_opts: EvmOpts,
    pub sender_nonce: U256,
    pub backend: Option<Backend>,
    pub called_function: Option<Function>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::LoadConfig;
    use foundry_cli_test_utils::tempfile::tempdir;
    use std::fs;

    #[test]
    fn can_parse_sig() {
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--sig",
            "0x522bb704000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfFFb92266",
        ]);
        assert_eq!(
            args.sig,
            "522bb704000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfFFb92266"
        );
    }

    #[test]
    fn can_merge_script_config() {
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--etherscan-api-key",
            "goerli",
        ]);
        let config = args.load_config();
        assert_eq!(config.etherscan_api_key, Some("goerli".to_string()));
    }

    #[test]
    fn can_extract_script_etherscan_key() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
                [profile.default]
                etherscan_api_key = "mumbai"

                [etherscan]
                mumbai = { key = "https://etherscan-mumbai.com/" }
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--etherscan-api-key",
            "mumbai",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);

        let config = args.load_config();
        let mumbai = config.get_etherscan_api_key(Some(ethers::types::Chain::PolygonMumbai));
        assert_eq!(mumbai, Some("https://etherscan-mumbai.com/".to_string()));
    }
}
