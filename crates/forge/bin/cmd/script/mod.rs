use self::{build::BuildOutput, runner::ScriptRunner};
use super::{build::BuildArgs, retry::RetryArgs};
use alloy_primitives::{Address, Bytes, U256};
use clap::{Parser, ValueHint};
use dialoguer::Confirm;
use ethers::{
    abi::{Abi, Function, HumanReadableParser},
    prelude::{
        artifacts::{ContractBytecodeSome, Libraries},
        ArtifactId, Project,
    },
    providers::{Http, Middleware},
    signers::LocalWallet,
    types::{
        transaction::eip2718::TypedTransaction, Chain, Log, NameOrAddress, TransactionRequest,
    },
};
use eyre::{ContextCompat, Result, WrapErr};
use forge::{
    debug::DebugArena,
    decode::decode_console_logs,
    executor::{opts::EvmOpts, Backend},
    trace::{
        identifier::{EtherscanIdentifier, LocalTraceIdentifier, SignaturesIdentifier},
        CallTraceDecoder, CallTraceDecoderBuilder, RawOrDecodedCall, RawOrDecodedReturnData,
        TraceKind, Traces,
    },
    CallKind,
};
use foundry_cli::opts::MultiWallet;
use foundry_common::{
    abi::{encode_function_args, format_token},
    contracts::get_contract_name,
    errors::UnlinkedByteCode,
    evm::{Breakpoints, EvmArgs},
    shell, ContractsByArtifact, RpcUrl, CONTRACT_MAX_SIZE, SELECTOR_LEN,
};
use foundry_compilers::contracts::ArtifactContracts;
use foundry_config::{
    figment,
    figment::{
        value::{Dict, Map},
        Metadata, Profile, Provider,
    },
    Config,
};
use foundry_evm::{
    decode,
    executor::inspector::{
        cheatcodes::{util::BroadcastableTransactions, BroadcastableTransaction},
        DEFAULT_CREATE2_DEPLOYER,
    },
};
use foundry_utils::types::{ToAlloy, ToEthers};
use futures::future;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use yansi::Paint;

mod artifacts;
mod broadcast;
mod build;
mod cmd;
mod executor;
mod multi;
mod providers;
mod receipts;
mod runner;
mod sequence;
pub mod transaction;
mod verify;

pub use transaction::TransactionWithMetadata;

/// List of Chains that support Shanghai.
static SHANGHAI_ENABLED_CHAINS: &[Chain] = &[
    // Ethereum Mainnet
    Chain::Mainnet,
    // Goerli
    Chain::Goerli,
    // Sepolia
    Chain::Sepolia,
];

// Loads project's figment and merges the build cli arguments into it
foundry_config::merge_impl_figment_convert!(ScriptArgs, opts, evm_opts);

/// CLI arguments for `forge script`.
#[derive(Debug, Clone, Parser, Default)]
pub struct ScriptArgs {
    /// The contract you want to run. Either the file path or contract name.
    ///
    /// If multiple contracts exist in the same file you must specify the target contract with
    /// --target-contract.
    #[clap(value_hint = ValueHint::FilePath)]
    pub path: String,

    /// Arguments to pass to the script function.
    pub args: Vec<String>,

    /// The name of the contract you want to run.
    #[clap(long, visible_alias = "tc", value_name = "CONTRACT_NAME")]
    pub target_contract: Option<String>,

    /// The signature of the function you want to call in the contract, or raw calldata.
    #[clap(
        long,
        short,
        default_value = "run()",
        value_parser = foundry_common::clap_helpers::strip_0x_prefix
    )]
    pub sig: String,

    /// Max priority fee per gas for EIP1559 transactions.
    #[clap(
        long,
        env = "ETH_PRIORITY_GAS_PRICE",
        value_parser = foundry_cli::utils::alloy_parse_ether_value,
        value_name = "PRICE"
    )]
    pub priority_gas_price: Option<U256>,

    /// Use legacy transactions instead of EIP1559 ones.
    ///
    /// This is auto-enabled for common networks without EIP1559.
    #[clap(long)]
    pub legacy: bool,

    /// Broadcasts the transactions.
    #[clap(long)]
    pub broadcast: bool,

    /// Skips on-chain simulation.
    #[clap(long)]
    pub skip_simulation: bool,

    /// Relative percentage to multiply gas estimates by.
    #[clap(long, short, default_value = "130")]
    pub gas_estimate_multiplier: u64,

    /// Send via `eth_sendTransaction` using the `--from` argument or `$ETH_FROM` as sender
    #[clap(
        long,
        requires = "sender",
        conflicts_with_all = &["private_key", "private_keys", "froms", "ledger", "trezor", "aws"],
    )]
    pub unlocked: bool,

    /// Resumes submitting transactions that failed or timed-out previously.
    ///
    /// It DOES NOT simulate the script again and it expects nonces to have remained the same.
    ///
    /// Example: If transaction N has a nonce of 22, then the account should have a nonce of 22,
    /// otherwise it fails.
    #[clap(long)]
    pub resume: bool,

    /// If present, --resume or --verify will be assumed to be a multi chain deployment.
    #[clap(long)]
    pub multi: bool,

    /// Open the script in the debugger.
    ///
    /// Takes precedence over broadcast.
    #[clap(long)]
    pub debug: bool,

    /// Makes sure a transaction is sent,
    /// only after its previous one has been confirmed and succeeded.
    #[clap(long)]
    pub slow: bool,

    /// Disables interactive prompts that might appear when deploying big contracts.
    ///
    /// For more info on the contract size limit, see EIP-170: <https://eips.ethereum.org/EIPS/eip-170>
    #[clap(long)]
    pub non_interactive: bool,

    /// The Etherscan (or equivalent) API key
    #[clap(long, env = "ETHERSCAN_API_KEY", value_name = "KEY")]
    pub etherscan_api_key: Option<String>,

    /// Verifies all the contracts found in the receipts of a script, if any.
    #[clap(long)]
    pub verify: bool,

    /// Output results in JSON format.
    #[clap(long)]
    pub json: bool,

    /// Gas price for legacy transactions, or max fee per gas for EIP1559 transactions.
    #[clap(
        long,
        env = "ETH_GAS_PRICE",
        value_parser = foundry_cli::utils::alloy_parse_ether_value,
        value_name = "PRICE",
    )]
    pub with_gas_price: Option<U256>,

    #[clap(flatten)]
    pub opts: BuildArgs,

    #[clap(flatten)]
    pub wallets: MultiWallet,

    #[clap(flatten)]
    pub evm_opts: EvmArgs,

    #[clap(flatten)]
    pub verifier: super::verify::VerifierArgs,

    #[clap(flatten)]
    pub retry: RetryArgs,
}

// === impl ScriptArgs ===

impl ScriptArgs {
    pub fn decode_traces(
        &self,
        script_config: &ScriptConfig,
        result: &mut ScriptResult,
        known_contracts: &ContractsByArtifact,
    ) -> Result<CallTraceDecoder> {
        let verbosity = script_config.evm_opts.verbosity;
        let mut etherscan_identifier = EtherscanIdentifier::new(
            &script_config.config,
            script_config.evm_opts.get_remote_chain_id(),
        )?;

        let mut local_identifier = LocalTraceIdentifier::new(known_contracts);
        let mut decoder = CallTraceDecoderBuilder::new()
            .with_labels(
                result.labeled_addresses.iter().map(|(a, s)| ((*a).to_ethers(), s.clone())),
            )
            .with_verbosity(verbosity)
            .with_signature_identifier(SignaturesIdentifier::new(
                Config::foundry_cache_dir(),
                script_config.config.offline,
            )?)
            .build();

        // Decoding traces using etherscan is costly as we run into rate limits,
        // causing scripts to run for a very long time unnecessarily.
        // Therefore, we only try and use etherscan if the user has provided an API key.
        let should_use_etherscan_traces = script_config.config.etherscan_api_key.is_some();

        for (_, trace) in &mut result.traces {
            decoder.identify(trace, &mut local_identifier);
            if should_use_etherscan_traces {
                decoder.identify(trace, &mut etherscan_identifier);
            }
        }
        Ok(decoder)
    }

    pub fn get_returns(
        &self,
        script_config: &ScriptConfig,
        returned: &Bytes,
    ) -> Result<HashMap<String, NestedValue>> {
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
                shell::println(format!("{returned:?}"))?;
            }
        }

        Ok(returns)
    }

    pub async fn show_traces(
        &self,
        script_config: &ScriptConfig,
        decoder: &CallTraceDecoder,
        result: &mut ScriptResult,
    ) -> Result<()> {
        let verbosity = script_config.evm_opts.verbosity;
        let func = script_config.called_function.as_ref().expect("There should be a function.");

        if !result.success || verbosity > 3 {
            if result.traces.is_empty() {
                eyre::bail!("Unexpected error: No traces despite verbosity level. Please report this as a bug: https://github.com/foundry-rs/foundry/issues/new?assignees=&labels=T-bug&template=BUG-FORM.yml");
            }

            shell::println("Traces:")?;
            for (kind, trace) in &mut result.traces {
                let should_include = match kind {
                    TraceKind::Setup => verbosity >= 5,
                    TraceKind::Execution => verbosity > 3,
                    _ => false,
                } || !result.success;

                if should_include {
                    decoder.decode(trace).await;
                    shell::println(format!("{trace}"))?;
                }
            }
            shell::println(String::new())?;
        }

        if result.success {
            shell::println(format!("{}", Paint::green("Script ran successfully.")))?;
        }

        if script_config.evm_opts.fork_url.is_none() {
            shell::println(format!("Gas used: {}", result.gas_used))?;
        }

        if result.success && !result.returned.is_empty() {
            shell::println("\n== Return ==")?;
            match func.decode_output(&result.returned) {
                Ok(decoded) => {
                    for (index, (token, output)) in decoded.iter().zip(&func.outputs).enumerate() {
                        let internal_type = output.internal_type.as_deref().unwrap_or("unknown");

                        let label = if !output.name.is_empty() {
                            output.name.to_string()
                        } else {
                            index.to_string()
                        };
                        shell::println(format!(
                            "{}: {internal_type} {}",
                            label.trim_end(),
                            format_token(token)
                        ))?;
                    }
                }
                Err(_) => {
                    shell::println(format!("{:x?}", (&result.returned)))?;
                }
            }
        }

        let console_logs = decode_console_logs(&result.logs);
        if !console_logs.is_empty() {
            shell::println("\n== Logs ==")?;
            for log in console_logs {
                shell::println(format!("  {log}"))?;
            }
        }

        if !result.success {
            let revert_msg = decode::decode_revert(&result.returned[..], None, None)
                .map(|err| format!("{err}\n"))
                .unwrap_or_else(|_| "Script failed.\n".to_string());

            eyre::bail!("{}", Paint::red(revert_msg));
        }

        Ok(())
    }

    pub fn show_json(&self, script_config: &ScriptConfig, result: &ScriptResult) -> Result<()> {
        let returns = self.get_returns(script_config, &result.returned)?;

        let console_logs = decode_console_logs(&result.logs);
        let output = JsonResult { logs: console_logs, gas_used: result.gas_used, returns };
        let j = serde_json::to_string(&output)?;
        shell::println(j)?;

        Ok(())
    }

    /// It finds the deployer from the running script and uses it to predeploy libraries.
    ///
    /// If there are multiple candidate addresses, it skips everything and lets `--sender` deploy
    /// them instead.
    fn maybe_new_sender(
        &self,
        evm_opts: &EvmOpts,
        transactions: Option<&BroadcastableTransactions>,
        predeploy_libraries: &[Bytes],
    ) -> Result<Option<Address>> {
        let mut new_sender = None;

        if let Some(txs) = transactions {
            // If the user passed a `--sender` don't check anything.
            if !predeploy_libraries.is_empty() && self.evm_opts.sender.is_none() {
                for tx in txs.iter() {
                    match &tx.transaction {
                        TypedTransaction::Legacy(tx) => {
                            if tx.to.is_none() {
                                let sender = tx.from.expect("no sender").to_alloy();
                                if let Some(ns) = new_sender {
                                    if sender != ns {
                                        shell::println("You have more than one deployer who could predeploy libraries. Using `--sender` instead.")?;
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
        nonce: u64,
        data: &[Bytes],
        fork_url: &Option<RpcUrl>,
    ) -> BroadcastableTransactions {
        data.iter()
            .enumerate()
            .map(|(i, bytes)| BroadcastableTransaction {
                rpc: fork_url.clone(),
                transaction: TypedTransaction::Legacy(TransactionRequest {
                    from: Some(from.to_ethers()),
                    data: Some(bytes.clone().0.into()),
                    nonce: Some(ethers::types::U256::from(nonce + i as u64)),
                    ..Default::default()
                }),
            })
            .collect()
    }

    /// Returns the Function and calldata based on the signature
    ///
    /// If the `sig` is a valid human-readable function we find the corresponding function in the
    /// `abi` If the `sig` is valid hex, we assume it's calldata and try to find the
    /// corresponding function by matching the selector, first 4 bytes in the calldata.
    ///
    /// Note: We assume that the `sig` is already stripped of its prefix, See [`ScriptArgs`]
    pub fn get_method_and_calldata(&self, abi: &Abi) -> Result<(Function, Bytes)> {
        let (func, data) = if let Ok(func) = HumanReadableParser::parse_function(&self.sig) {
            (
                abi.functions().find(|&abi_func| abi_func.selector() == func.selector()).wrap_err(
                    format!("Function `{}` is not implemented in your script.", self.sig),
                )?,
                encode_function_args(&func, &self.args)?.into(),
            )
        } else {
            let decoded = hex::decode(&self.sig).wrap_err("Invalid hex calldata")?;
            let selector = &decoded[..SELECTOR_LEN];
            (
                abi.functions().find(|&func| selector == &func.selector()[..]).ok_or_else(
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

    /// Checks if the transaction is a deployment with either a size above the `CONTRACT_MAX_SIZE`
    /// or specified `code_size_limit`.
    ///
    /// If `self.broadcast` is enabled, it asks confirmation of the user. Otherwise, it just warns
    /// the user.
    fn check_contract_sizes(
        &self,
        result: &ScriptResult,
        known_contracts: &BTreeMap<ArtifactId, ContractBytecodeSome>,
    ) -> Result<()> {
        // (name, &init, &deployed)[]
        let mut bytecodes: Vec<(String, &[u8], &[u8])> = vec![];

        // From artifacts
        for (artifact, bytecode) in known_contracts.iter() {
            if bytecode.bytecode.object.is_unlinked() {
                return Err(UnlinkedByteCode::Bytecode(artifact.identifier()).into())
            }
            let init_code = bytecode.bytecode.object.as_bytes().unwrap();
            // Ignore abstract contracts
            if let Some(ref deployed_code) = bytecode.deployed_bytecode.bytecode {
                if deployed_code.object.is_unlinked() {
                    return Err(UnlinkedByteCode::DeployedBytecode(artifact.identifier()).into())
                }
                let deployed_code = deployed_code.object.as_bytes().unwrap();
                bytecodes.push((artifact.name.clone(), init_code, deployed_code));
            }
        }

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
                    if !bytecodes.iter().any(|(_, b, _)| *b == init_code.as_ref()) {
                        bytecodes.push((format!("Unknown{unknown_c}"), init_code, deployed_code));
                        unknown_c += 1;
                    }
                    continue
                }
            }
            // Both should be raw and not decoded since it's just bytecode
            eyre::bail!("Create node returned decoded data: {:?}", node);
        }

        let mut prompt_user = false;
        let max_size = match self.evm_opts.env.code_size_limit {
            Some(size) => size,
            None => CONTRACT_MAX_SIZE,
        };

        for (data, to) in result.transactions.iter().flat_map(|txes| {
            txes.iter().filter_map(|tx| {
                tx.transaction
                    .data()
                    .filter(|data| data.len() > max_size)
                    .map(|data| (data, tx.transaction.to()))
            })
        }) {
            let mut offset = 0;

            // Find if it's a CREATE or CREATE2. Otherwise, skip transaction.
            if let Some(NameOrAddress::Address(to)) = to {
                if to.to_alloy() == DEFAULT_CREATE2_DEPLOYER {
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

                if deployment_size > max_size {
                    prompt_user = self.broadcast;
                    shell::println(format!(
                        "{}",
                        Paint::red(format!(
                            "`{name}` is above the contract size limit ({deployment_size} > {max_size})."
                        ))
                    ))?;
                }
            }
        }

        // Only prompt if we're broadcasting and we've not disabled interactivity.
        if prompt_user &&
            !self.non_interactive &&
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

#[derive(Default)]
pub struct ScriptResult {
    pub success: bool,
    pub logs: Vec<Log>,
    pub traces: Traces,
    pub debug: Option<Vec<DebugArena>>,
    pub gas_used: u64,
    pub labeled_addresses: BTreeMap<Address, String>,
    pub transactions: Option<BroadcastableTransactions>,
    pub returned: Bytes,
    pub address: Option<Address>,
    pub script_wallets: Vec<LocalWallet>,
    pub breakpoints: Breakpoints,
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

#[derive(Clone, Debug, Default)]
pub struct ScriptConfig {
    pub config: Config,
    pub evm_opts: EvmOpts,
    pub sender_nonce: u64,
    /// Maps a rpc url to a backend
    pub backends: HashMap<RpcUrl, Backend>,
    /// Script target contract
    pub target_contract: Option<ArtifactId>,
    /// Function called by the script
    pub called_function: Option<Function>,
    /// Unique list of rpc urls present
    pub total_rpcs: HashSet<RpcUrl>,
    /// If true, one of the transactions did not have a rpc
    pub missing_rpc: bool,
    /// Should return some debug information
    pub debug: bool,
}

impl ScriptConfig {
    fn collect_rpcs(&mut self, txs: &BroadcastableTransactions) {
        self.missing_rpc = txs.iter().any(|tx| tx.rpc.is_none());

        self.total_rpcs
            .extend(txs.iter().filter_map(|tx| tx.rpc.as_ref().cloned()).collect::<HashSet<_>>());

        if let Some(rpc) = &self.evm_opts.fork_url {
            self.total_rpcs.insert(rpc.clone());
        }
    }

    fn has_multiple_rpcs(&self) -> bool {
        self.total_rpcs.len() > 1
    }

    /// Certain features are disabled for multi chain deployments, and if tried, will return
    /// error. [library support]
    fn check_multi_chain_constraints(&self, libraries: &Libraries) -> Result<()> {
        if self.has_multiple_rpcs() || (self.missing_rpc && !self.total_rpcs.is_empty()) {
            shell::eprintln(format!(
                "{}",
                Paint::yellow(
                    "Multi chain deployment is still under development. Use with caution."
                )
            ))?;
            if !libraries.libs.is_empty() {
                eyre::bail!(
                    "Multi chain deployment does not support library linking at the moment."
                )
            }
        }
        Ok(())
    }

    /// Returns the script target contract
    fn target_contract(&self) -> &ArtifactId {
        self.target_contract.as_ref().expect("should exist after building")
    }

    /// Checks if the RPCs used point to chains that support EIP-3855.
    /// If not, warns the user.
    async fn check_shanghai_support(&self) -> Result<()> {
        let chain_ids = self.total_rpcs.iter().map(|rpc| async move {
            if let Ok(provider) = ethers::providers::Provider::<Http>::try_from(rpc) {
                match provider.get_chainid().await {
                    Ok(chain_id) => match TryInto::<Chain>::try_into(chain_id) {
                        Ok(chain) => return Some((SHANGHAI_ENABLED_CHAINS.contains(&chain), chain)),
                        Err(_) => return None,
                    },
                    Err(_) => return None,
                }
            }
            None
        });

        let chain_ids: Vec<_> = future::join_all(chain_ids).await.into_iter().flatten().collect();

        let chain_id_unsupported = chain_ids.iter().any(|(supported, _)| !supported);

        // At least one chain ID is unsupported, therefore we print the message.
        if chain_id_unsupported {
            let msg = format!(
                r#"
EIP-3855 is not supported in one or more of the RPCs used.
Unsupported Chain IDs: {}.
Contracts deployed with a Solidity version equal or higher than 0.8.20 might not work properly.
For more information, please see https://eips.ethereum.org/EIPS/eip-3855"#,
                chain_ids
                    .iter()
                    .filter(|(supported, _)| !supported)
                    .map(|(_, chain)| format!("{}", *chain as u64))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            shell::println(Paint::yellow(msg))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_cli::utils::LoadConfig;
    use foundry_config::UnresolvedEnvVarError;
    use foundry_test_utils::tempfile::tempdir;
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
    fn can_parse_unlocked() {
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "Contract.sol",
            "--sender",
            "0x4e59b44847b379578588920ca78fbf26c0b4956c",
            "--unlocked",
        ]);
        assert!(args.unlocked);

        let key = U256::ZERO;
        let args = ScriptArgs::try_parse_from([
            "foundry-cli",
            "Contract.sol",
            "--sender",
            "0x4e59b44847b379578588920ca78fbf26c0b4956c",
            "--unlocked",
            "--private-key",
            key.to_string().as_str(),
        ]);
        assert!(args.is_err());
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
    fn can_parse_verifier_url() {
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "script",
            "script/Test.s.sol:TestScript",
            "--fork-url",
            "http://localhost:8545",
            "--verifier-url",
            "http://localhost:3000/api/verify",
            "--etherscan-api-key",
            "blacksmith",
            "--broadcast",
            "--verify",
            "-vvvv",
        ]);
        assert_eq!(
            args.verifier.verifier_url,
            Some("http://localhost:3000/api/verify".to_string())
        );
    }

    #[test]
    fn can_extract_code_size_limit() {
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "script",
            "script/Test.s.sol:TestScript",
            "--fork-url",
            "http://localhost:8545",
            "--broadcast",
            "--code-size-limit",
            "50000",
        ]);
        assert_eq!(args.evm_opts.env.code_size_limit, Some(50000));
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

    #[test]
    fn can_extract_script_rpc_alias() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
                [profile.default]

                [rpc_endpoints]
                polygonMumbai = "https://polygon-mumbai.g.alchemy.com/v2/${_CAN_EXTRACT_RPC_ALIAS}"
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "DeployV1",
            "--rpc-url",
            "polygonMumbai",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);

        let err = args.load_config_and_evm_opts().unwrap_err();

        assert!(err.downcast::<UnresolvedEnvVarError>().is_ok());

        std::env::set_var("_CAN_EXTRACT_RPC_ALIAS", "123456");
        let (config, evm_opts) = args.load_config_and_evm_opts().unwrap();
        assert_eq!(config.eth_rpc_url, Some("polygonMumbai".to_string()));
        assert_eq!(
            evm_opts.fork_url,
            Some("https://polygon-mumbai.g.alchemy.com/v2/123456".to_string())
        );
    }

    #[test]
    fn can_extract_script_rpc_and_etherscan_alias() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
            [profile.default]

            [rpc_endpoints]
            mumbai = "https://polygon-mumbai.g.alchemy.com/v2/${_EXTRACT_RPC_ALIAS}"

            [etherscan]
            mumbai = { key = "${_POLYSCAN_API_KEY}", chain = 80001, url = "https://api-testnet.polygonscan.com/" }
        "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "DeployV1",
            "--rpc-url",
            "mumbai",
            "--etherscan-api-key",
            "mumbai",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);
        let err = args.load_config_and_evm_opts().unwrap_err();

        assert!(err.downcast::<UnresolvedEnvVarError>().is_ok());

        std::env::set_var("_EXTRACT_RPC_ALIAS", "123456");
        std::env::set_var("_POLYSCAN_API_KEY", "polygonkey");
        let (config, evm_opts) = args.load_config_and_evm_opts().unwrap();
        assert_eq!(config.eth_rpc_url, Some("mumbai".to_string()));
        assert_eq!(
            evm_opts.fork_url,
            Some("https://polygon-mumbai.g.alchemy.com/v2/123456".to_string())
        );
        let etherscan = config.get_etherscan_api_key(Some(80001u64));
        assert_eq!(etherscan, Some("polygonkey".to_string()));
        let etherscan = config.get_etherscan_api_key(Option::<u64>::None);
        assert_eq!(etherscan, Some("polygonkey".to_string()));
    }

    #[test]
    fn can_extract_script_rpc_and_sole_etherscan_alias() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config = r#"
                [profile.default]

               [rpc_endpoints]
                mumbai = "https://polygon-mumbai.g.alchemy.com/v2/${_SOLE_EXTRACT_RPC_ALIAS}"

                [etherscan]
                mumbai = { key = "${_SOLE_POLYSCAN_API_KEY}" }
            "#;

        let toml_file = root.join(Config::FILE_NAME);
        fs::write(toml_file, config).unwrap();
        let args: ScriptArgs = ScriptArgs::parse_from([
            "foundry-cli",
            "DeployV1",
            "--rpc-url",
            "mumbai",
            "--root",
            root.as_os_str().to_str().unwrap(),
        ]);
        let err = args.load_config_and_evm_opts().unwrap_err();

        assert!(err.downcast::<UnresolvedEnvVarError>().is_ok());

        std::env::set_var("_SOLE_EXTRACT_RPC_ALIAS", "123456");
        std::env::set_var("_SOLE_POLYSCAN_API_KEY", "polygonkey");
        let (config, evm_opts) = args.load_config_and_evm_opts().unwrap();
        assert_eq!(
            evm_opts.fork_url,
            Some("https://polygon-mumbai.g.alchemy.com/v2/123456".to_string())
        );
        let etherscan = config.get_etherscan_api_key(Some(80001u64));
        assert_eq!(etherscan, Some("polygonkey".to_string()));
        let etherscan = config.get_etherscan_api_key(Option::<u64>::None);
        assert_eq!(etherscan, Some("polygonkey".to_string()));
    }

    // <https://github.com/foundry-rs/foundry/issues/5923>
    #[test]
    fn test_5923() {
        let args: ScriptArgs =
            ScriptArgs::parse_from(["foundry-cli", "DeployV1", "--priority-gas-price", "100"]);
        assert!(args.priority_gas_price.is_some());
    }
}
