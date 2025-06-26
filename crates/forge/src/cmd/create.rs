use crate::cmd::install;
use alloy_chains::Chain;
use alloy_dyn_abi::{DynSolValue, JsonAbiExt, Specifier};
use alloy_json_abi::{Constructor, JsonAbi};
use alloy_network::{AnyNetwork, AnyTransactionReceipt, EthereumWallet, TransactionBuilder};
use alloy_primitives::{hex, Address, Bytes};
use alloy_provider::{PendingTransactionError, Provider, ProviderBuilder};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use alloy_signer::Signer;
use alloy_transport::TransportError;
use clap::{Parser, ValueHint};

use codec::{Compact, Encode};
use eyre::{Context, Result};
use forge_verify::{RetryArgs, VerifierArgs, VerifyArgs};
use foundry_cli::{
    opts::{BuildOpts, EthereumOpts, EtherscanOpts, TransactionOpts},
    utils::{self, read_constructor_args_file, remove_contract, LoadConfig},
};
use foundry_common::{
    compile::{self},
    fmt::parse_tokens,
    shell,
};
use foundry_compilers::{
    artifacts::BytecodeObject, info::ContractInfo, utils::canonicalize, ArtifactId,
};
use foundry_config::{
    figment::{
        self,
        value::{Dict, Map},
        Metadata, Profile,
    },
    merge_impl_figment_convert, Config,
};
use serde_json::json;
use std::collections::HashSet;
use std::{borrow::Borrow, marker::PhantomData, path::PathBuf, sync::Arc, time::Duration};

merge_impl_figment_convert!(CreateArgs, build, eth);

/// CLI arguments for `forge create`.
#[derive(Clone, Debug, Parser)]
pub struct CreateArgs {
    /// The contract identifier in the form `<path>:<contractname>`.
    contract: ContractInfo,

    /// The constructor arguments.
    #[arg(
        long,
        num_args(1..),
        conflicts_with = "constructor_args_path",
        value_name = "ARGS",
        allow_hyphen_values = true,
    )]
    constructor_args: Vec<String>,

    /// The path to a file containing the constructor arguments.
    #[arg(
        long,
        value_hint = ValueHint::FilePath,
        value_name = "PATH",
    )]
    constructor_args_path: Option<PathBuf>,

    /// Broadcast the transaction.
    #[arg(long)]
    pub broadcast: bool,

    /// Verify contract after creation.
    #[arg(long)]
    verify: bool,

    /// Send via `eth_sendTransaction` using the `--from` argument or `$ETH_FROM` as sender
    #[arg(long, requires = "from")]
    unlocked: bool,

    /// Prints the standard json compiler input if `--verify` is provided.
    ///
    /// The standard json compiler input can be used to manually submit contract verification in
    /// the browser.
    #[arg(long, requires = "verify")]
    show_standard_json_input: bool,

    /// Timeout to use for broadcasting transactions.
    #[arg(long, env = "ETH_TIMEOUT")]
    pub timeout: Option<u64>,

    #[command(flatten)]
    build: BuildOpts,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,

    #[command(flatten)]
    pub verifier: VerifierArgs,

    #[command(flatten)]
    retry: RetryArgs,
}

/// Finds all contracts being initialized in the target contract along with their file paths
///
/// # Arguments
/// * `target_path` - Path to the contract file being analyzed
/// * `output` - The compilation output containing all contract information
///
/// # Returns
/// * `Result<Vec<(String, PathBuf)>>` - Vector of (contract_name, file_path) tuples
pub fn find_initialized_contracts_with_paths(
    target_path: &PathBuf,
    output: &foundry_compilers::ProjectCompileOutput,
) -> Result<Vec<(String, PathBuf)>> {
    let mut initialized_contracts = Vec::new();

    // Read the source file content
    let source_content = std::fs::read_to_string(target_path)
        .wrap_err_with(|| format!("Failed to read target file: {}", target_path.display()))?;

    // Parse the Solidity source code
    let (parse_tree, _diagnostics) = match solang_parser::parse(&source_content, 0) {
        Ok((tree, diag)) => (tree, diag),
        Err(_diagnostics) => {
            // If parsing fails, return empty list
            return Ok(vec![]);
        }
    };

    // Find all contract names being initialized
    let mut initialized_names = HashSet::new();
    find_contract_initializations(&parse_tree.0, &mut initialized_names);

    // Map contract names to their file paths using compilation output
    let contracts = &output.output().contracts;
    for contract_name in initialized_names {
        // Search through all files to find where this contract is defined
        for (file_path, file_contracts) in contracts.iter() {
            if file_contracts.contains_key(&contract_name) {
                initialized_contracts.push((contract_name.clone(), file_path.clone()));
                break;
            }
        }
    }

    initialized_contracts.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(initialized_contracts)
}

/// Recursively searches AST for contract initializations (new ContractName() expressions)
fn find_contract_initializations(
    source_unit_parts: &[solang_parser::pt::SourceUnitPart],
    initialized_names: &mut HashSet<String>,
) {
    use solang_parser::pt::*;

    for part in source_unit_parts.iter() {
        match part {
            SourceUnitPart::ContractDefinition(contract) => {
                // Look for initializations within contract functions
                for contract_part in contract.parts.iter() {
                    match contract_part {
                        ContractPart::FunctionDefinition(func) => {
                            if let Some(Statement::Block { statements, .. }) = &func.body {
                                find_initializations_in_statements(statements, initialized_names);
                            }
                        }
                        ContractPart::VariableDefinition(var_def) => {
                            // Check for contract initializations in variable definitions
                            if let Some(expr) = &var_def.initializer {
                                find_initializations_in_expression(expr, initialized_names);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// Recursively searches statements for contract initializations
fn find_initializations_in_statements(
    statements: &[solang_parser::pt::Statement],
    initialized_names: &mut HashSet<String>,
) {
    use solang_parser::pt::*;

    for stmt in statements.iter() {
        match stmt {
            Statement::Expression(_, expr) => {
                find_initializations_in_expression(expr, initialized_names);
            }
            Statement::VariableDefinition(_, _var_decl, init_expr) => {
                if let Some(expr) = init_expr {
                    find_initializations_in_expression(expr, initialized_names);
                }
            }
            Statement::Block { statements, .. } => {
                find_initializations_in_statements(statements, initialized_names);
            }
            _ => {}
        }
    }
}

/// Searches expressions for contract initializations (new ContractName() calls)
fn find_initializations_in_expression(
    expr: &solang_parser::pt::Expression,
    initialized_names: &mut HashSet<String>,
) {
    use solang_parser::pt::*;

    match expr {
        Expression::New(_, type_expr) => {
            // Extract contract name from the type expression
            let contract_name = extract_contract_name_from_expression(type_expr);
            if !contract_name.is_empty() {
                initialized_names.insert(contract_name);
            }
        }
        Expression::FunctionCall(_, func_expr, args) => {
            // Recursively check function call expressions and arguments
            find_initializations_in_expression(func_expr, initialized_names);
            for arg in args.iter() {
                find_initializations_in_expression(arg, initialized_names);
            }
        }
        Expression::Assign(_, left_expr, right_expr) => {
            find_initializations_in_expression(left_expr, initialized_names);
            find_initializations_in_expression(right_expr, initialized_names);
        }
        Expression::MemberAccess(_, base_expr, _member) => {
            find_initializations_in_expression(base_expr, initialized_names);
        }
        _ => {}
    }
}

/// Extracts contract name from an Expression (for new Contract() patterns)
fn extract_contract_name_from_expression(expr: &solang_parser::pt::Expression) -> String {
    use solang_parser::pt::*;

    match expr {
        Expression::Variable(identifier) => identifier.name.clone(),
        Expression::MemberAccess(_, _, identifier) => identifier.name.clone(),
        Expression::FunctionCall(_, func_expr, _args) => {
            // Handle cases like ContractName() where the contract name is in the function expression
            extract_contract_name_from_expression(func_expr)
        }
        Expression::Type(_, ty) => extract_contract_name_from_type(ty),
        _ => String::new(),
    }
}

/// Extracts contract name from a Type
fn extract_contract_name_from_type(ty: &solang_parser::pt::Type) -> String {
    use solang_parser::pt::*;

    match ty {
        Type::Address
        | Type::AddressPayable
        | Type::Bool
        | Type::String
        | Type::Int(_)
        | Type::Uint(_)
        | Type::Bytes(_)
        | Type::DynamicBytes
        | Type::Mapping { .. }
        | Type::Function { .. } => String::new(),
        _ => {
            // For custom types (likely contracts), try to extract the identifier
            let type_str = format!("{:?}", ty);

            // Look for identifier patterns in the debug output
            if let Some(start) = type_str.find("name: \"") {
                let start = start + 7; // Skip 'name: "'
                if let Some(end) = type_str[start..].find('"') {
                    let name = &type_str[start..start + end];
                    return name.to_string();
                }
            }

            // Fallback: look for capitalized identifiers
            type_str
                .split_whitespace()
                .find(|s| s.chars().next().map_or(false, |c| c.is_ascii_uppercase()) && s.len() > 1)
                .unwrap_or("")
                .trim_matches(',')
                .trim_matches(')')
                .to_string()
        }
    }
}

async fn upload_child_contract_alloy(
    rpc_url: Option<String>,
    private_key: Option<String>,
    encoded_bytes: String,
) -> Result<String> {
    use alloy_primitives::{Address, U256};
    use alloy_provider::Provider;
    use alloy_rpc_types::TransactionRequest;
    use alloy_signer_local::PrivateKeySigner;
    use alloy_serde::WithOtherFields;
    use foundry_common::provider::ProviderBuilder;
    use std::str::FromStr;

    // Use provided RPC URL or fallback to default
    let rpc_url = rpc_url.unwrap_or_else(|| "https://testnet-passet-hub-eth-rpc.polkadot.io/".to_string());
    
    // Use provided private key or fallback to default
    let private_key = private_key.unwrap_or_else(|| {
        "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133".to_string()
    });

    // 1. Create wallet from private key
    let wallet = PrivateKeySigner::from_str(&private_key)?;

    // 2. Create provider with wallet using the proper Foundry pattern
    let provider = ProviderBuilder::new(&rpc_url)
        .build_with_wallet(EthereumWallet::new(wallet))?;

    // 3. Build transaction
    let magic_address: Address = "0x6d6f646c70792f70616464720000000000000000".parse()?;
    
    // Convert hex string to bytes for input
    let input_bytes = hex::decode(encoded_bytes.trim_start_matches("0x"))?;
    
    let tx = TransactionRequest::default()
        .to(magic_address)
        .input(input_bytes.into())
        .value(U256::from(0u64));

    // 4. Sign and send transaction
    let wrapped_tx = WithOtherFields::new(tx);
    let pending_tx = provider.send_transaction(wrapped_tx).await?;
    let receipt = pending_tx.get_receipt().await?;
    
    println!("Transaction sent! Hash: {:?}", receipt.transaction_hash);
    Ok(receipt.transaction_hash.to_string())
}

async fn upload_child_contract(
    rpc_url: Option<String>,
    private_key: Option<String>,
    code: String
) -> Result<String> {
    use std::process::Command;

    // Use provided RPC URL or fallback to default
    let rpc_url =
        rpc_url.unwrap_or_else(|| "https://testnet-passet-hub-eth-rpc.polkadot.io/".to_string());

    // Use provided private key or fallback to default
    let private_key = private_key.unwrap_or_else(|| {
        "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133".to_string()
    });

    // Build the cast send command
    let mut cmd = Command::new("cast");
    cmd.arg("send");
    cmd.arg("0x6d6f646c70792f70616464720000000000000000");
    cmd.arg("--rpc-url").arg(&rpc_url);
    cmd.arg("--private-key").arg(&private_key);
    cmd.arg(&code);

    println!("Executing command: {:?}", cmd);

    // Execute the command
    let output = cmd.output().map_err(|e| eyre::eyre!("Failed to execute cast send: {}", e))?;

    if output.status.success() {
        let tx_hash = String::from_utf8(output.stdout)
            .map_err(|e| eyre::eyre!("Failed to parse cast output: {}", e))?
            .trim()
            .to_string();
        println!("Transaction successful: {}", tx_hash);
        Ok(tx_hash)
    } else {
        let error =
            String::from_utf8(output.stderr).unwrap_or_else(|_| "Unknown error".to_string());
        println!("Command failed with error: {}", error);
        Err(eyre::eyre!("Cast send failed: {}", error))
    }
}

impl CreateArgs {
    /// Executes the command to create a contract
    pub async fn run(mut self) -> Result<()> {
        let mut config = self.load_config()?;
        println!("=== User Inputs ===");
        println!("RPC URL: {:?}", self.eth.rpc.url);
        println!("Private Key: {:?}", self.eth.wallet.raw.private_key);
        println!("==================");
        // Install missing dependencies.
        if install::install_missing_dependencies(&mut config) && config.auto_detect_remappings {
            // need to re-configure here to also catch additional remappings
            config = self.load_config()?;
        }

        // Find Project & Compile
        let project = config.project()?;

        let target_path = if let Some(ref mut path) = self.contract.path {
            canonicalize(project.root().join(path))?
        } else {
            project.find_contract_path(&self.contract.name)?
        };

        let output: foundry_compilers::ProjectCompileOutput =
            compile::compile_target(&target_path, &project, shell::is_json())?;

        // Find all contracts being initialized in the target contract along with their paths
        let initialized_contracts = find_initialized_contracts_with_paths(&target_path, &output)?;
        if !initialized_contracts.is_empty() {
            println!("Contracts being initialized in {}:", target_path.display());
            for (contract_name, contract_path) in &initialized_contracts {
                println!("  - {} (defined in: {})", contract_name, contract_path.display());

                // Try to get bytecode information for this contract
                if let Ok((_, bin, _)) =
                    remove_contract(output.clone(), contract_path, contract_name)
                {
                    match &bin.object {
                        BytecodeObject::Bytecode(bytes) => {
                            println!("    Bytecode: Available ({} bytes)", bytes.len());
                            println!("    Bytecode: 0x{}", hex::encode(bytes));
                            let scaled_encoded_bytes = bytes.encode();

                            let storage_deposit_limit = Compact(5378900000u128);
                            let encoded_storage_deposit_limit = storage_deposit_limit.encode();
                            let combined_hex = "0x3c04".to_string()
                                + &hex::encode(&scaled_encoded_bytes)
                                + &hex::encode(&encoded_storage_deposit_limit);
                            println!(
                                "    SCALE Encoded Bytecode: {}",
                                &combined_hex
                            );
                            // Pass RPC URL and private key to upload_child_contract
                            let rpc_url = self.eth.rpc.url.clone();
                            let private_key = self.eth.wallet.raw.private_key.clone();
                            //upload_child_contract(rpc_url, private_key, combined_hex).await;
                            upload_child_contract(rpc_url, private_key, combined_hex).await?;
                        }
                        BytecodeObject::Unlinked(_) => {
                            println!("    Bytecode: Available (unlinked)");
                        }
                    }
                } else {
                    println!("    Bytecode: Not available or compilation error");
                }
                //println!();
            }
        }

        let (abi, bin, id) = remove_contract(output, &target_path, &self.contract.name)?;

        let bin = match bin.object {
            BytecodeObject::Bytecode(_) => bin.object,
            _ => {
                let link_refs = bin
                    .link_references
                    .iter()
                    .flat_map(|(path, names)| {
                        names.keys().map(move |name| format!("\t{name}: {path}"))
                    })
                    .collect::<Vec<String>>()
                    .join("\n");
                eyre::bail!("Dynamic linking not supported in `create` command - deploy the following library contracts first, then provide the address to link at compile time\n{}", link_refs)
            }
        };

        // Add arguments to constructor
        let params = if let Some(constructor) = &abi.constructor {
            let constructor_args =
                self.constructor_args_path.clone().map(read_constructor_args_file).transpose()?;
            self.parse_constructor_args(
                constructor,
                constructor_args.as_deref().unwrap_or(&self.constructor_args),
            )?
        } else {
            vec![]
        };

        let provider = utils::get_provider(&config)?;

        // respect chain, if set explicitly via cmd args
        let chain_id = if let Some(chain_id) = self.chain_id() {
            chain_id
        } else {
            provider.get_chain_id().await?
        };

        // Whether to broadcast the transaction or not
        let dry_run = !self.broadcast;

        if self.unlocked {
            // Deploy with unlocked account
            let sender = self.eth.wallet.from.expect("required");
            self.deploy(
                abi,
                bin,
                params,
                provider,
                chain_id,
                sender,
                config.transaction_timeout,
                id,
                dry_run,
            )
            .await
        } else {
            // Deploy with signer
            let signer = self.eth.wallet.signer().await?;
            let deployer = signer.address();
            let provider = ProviderBuilder::<_, _, AnyNetwork>::default()
                .wallet(EthereumWallet::new(signer))
                .on_provider(provider);
            self.deploy(
                abi,
                bin,
                params,
                provider,
                chain_id,
                deployer,
                config.transaction_timeout,
                id,
                dry_run,
            )
            .await
        }
    }

    /// Returns the provided chain id, if any.
    fn chain_id(&self) -> Option<u64> {
        self.eth.etherscan.chain.map(|chain| chain.id())
    }

    /// Ensures the verify command can be executed.
    ///
    /// This is supposed to check any things that might go wrong when preparing a verify request
    /// before the contract is deployed. This should prevent situations where a contract is deployed
    /// successfully, but we fail to prepare a verify request which would require manual
    /// verification.
    async fn verify_preflight_check(
        &self,
        constructor_args: Option<String>,
        chain: u64,
        id: &ArtifactId,
    ) -> Result<()> {
        // NOTE: this does not represent the same `VerifyArgs` that would be sent after deployment,
        // since we don't know the address yet.
        let mut verify = VerifyArgs {
            address: Default::default(),
            contract: Some(self.contract.clone()),
            compiler_version: Some(id.version.to_string()),
            constructor_args,
            constructor_args_path: None,
            num_of_optimizations: None,
            etherscan: EtherscanOpts {
                key: self.eth.etherscan.key.clone(),
                chain: Some(chain.into()),
            },
            rpc: Default::default(),
            flatten: false,
            force: false,
            skip_is_verified_check: true,
            watch: true,
            retry: self.retry,
            libraries: self.build.libraries.clone(),
            root: None,
            verifier: self.verifier.clone(),
            via_ir: self.build.via_ir,
            evm_version: self.build.compiler.evm_version,
            show_standard_json_input: self.show_standard_json_input,
            guess_constructor_args: false,
            compilation_profile: Some(id.profile.to_string()),
        };

        // Check config for Etherscan API Keys to avoid preflight check failing if no
        // ETHERSCAN_API_KEY value set.
        let config = verify.load_config()?;
        verify.etherscan.key =
            config.get_etherscan_config_with_chain(Some(chain.into()))?.map(|c| c.key);

        let context = verify.resolve_context().await?;

        verify.verification_provider()?.preflight_verify_check(verify, context).await?;
        Ok(())
    }

    /// Deploys the contract
    #[expect(clippy::too_many_arguments)]
    async fn deploy<P: Provider<AnyNetwork>>(
        self,
        abi: JsonAbi,
        bin: BytecodeObject,
        args: Vec<DynSolValue>,
        provider: P,
        chain: u64,
        deployer_address: Address,
        timeout: u64,
        id: ArtifactId,
        dry_run: bool,
    ) -> Result<()> {
        let bin = bin.into_bytes().unwrap_or_default();
        if bin.is_empty() {
            eyre::bail!("no bytecode found in bin object for {}", self.contract.name)
        }

        let provider = Arc::new(provider);
        let factory = ContractFactory::new(abi.clone(), bin.clone(), provider.clone(), timeout);

        let is_args_empty = args.is_empty();
        let mut deployer =
            factory.deploy_tokens(args.clone()).context("failed to deploy contract").map_err(|e| {
                if is_args_empty {
                    e.wrap_err("no arguments provided for contract constructor; consider --constructor-args or --constructor-args-path")
                } else {
                    e
                }
            })?;
        let is_legacy = self.tx.legacy || Chain::from(chain).is_legacy();

        deployer.tx.set_from(deployer_address);
        deployer.tx.set_chain_id(chain);
        // `to` field must be set explicitly, cannot be None.
        if deployer.tx.to.is_none() {
            deployer.tx.set_create();
        }
        deployer.tx.set_nonce(if let Some(nonce) = self.tx.nonce {
            Ok(nonce.to())
        } else {
            provider.get_transaction_count(deployer_address).await
        }?);

        // set tx value if specified
        if let Some(value) = self.tx.value {
            deployer.tx.set_value(value);
        }

        deployer.tx.set_gas_limit(if let Some(gas_limit) = self.tx.gas_limit {
            Ok(gas_limit.to())
        } else {
            provider.estimate_gas(deployer.tx.clone()).await
        }?);

        if is_legacy {
            let gas_price = if let Some(gas_price) = self.tx.gas_price {
                gas_price.to()
            } else {
                provider.get_gas_price().await?
            };
            deployer.tx.set_gas_price(gas_price);
        } else {
            let estimate = provider.estimate_eip1559_fees().await.wrap_err("Failed to estimate EIP1559 fees. This chain might not support EIP1559, try adding --legacy to your command.")?;
            let priority_fee = if let Some(priority_fee) = self.tx.priority_gas_price {
                priority_fee.to()
            } else {
                estimate.max_priority_fee_per_gas
            };
            let max_fee = if let Some(max_fee) = self.tx.gas_price {
                max_fee.to()
            } else {
                estimate.max_fee_per_gas
            };

            deployer.tx.set_max_fee_per_gas(max_fee);
            deployer.tx.set_max_priority_fee_per_gas(priority_fee);
        }

        // Before we actually deploy the contract we try check if the verify settings are valid
        let mut constructor_args = None;
        if self.verify {
            if !args.is_empty() {
                let encoded_args = abi
                    .constructor()
                    .ok_or_else(|| eyre::eyre!("could not find constructor"))?
                    .abi_encode_input(&args)?;
                constructor_args = Some(hex::encode(encoded_args));
            }

            self.verify_preflight_check(constructor_args.clone(), chain, &id).await?;
        }

        if dry_run {
            if !shell::is_json() {
                sh_warn!("Dry run enabled, not broadcasting transaction\n")?;

                sh_println!("Contract: {}", self.contract.name)?;
                sh_println!(
                    "Transaction: {}",
                    serde_json::to_string_pretty(&deployer.tx.clone())?
                )?;
                sh_println!("ABI: {}\n", serde_json::to_string_pretty(&abi)?)?;

                sh_warn!("To broadcast this transaction, add --broadcast to the previous command. See forge create --help for more.")?;
            } else {
                let output = json!({
                    "contract": self.contract.name,
                    "transaction": &deployer.tx,
                    "abi":&abi
                });
                sh_println!("{}", serde_json::to_string_pretty(&output)?)?;
            }

            return Ok(());
        }

        // Deploy the actual contract
        let (deployed_contract, receipt) = deployer.send_with_receipt().await?;

        let address = deployed_contract;
        if shell::is_json() {
            let output = json!({
                "deployer": deployer_address.to_string(),
                "deployedTo": address.to_string(),
                "transactionHash": receipt.transaction_hash
            });
            sh_println!("{}", serde_json::to_string_pretty(&output)?)?;
        } else {
            sh_println!("Deployer: {deployer_address}")?;
            sh_println!("Deployed to: {address}")?;
            sh_println!("Transaction hash: {:?}", receipt.transaction_hash)?;
        };

        if !self.verify {
            return Ok(());
        }

        sh_println!("Starting contract verification...")?;

        let num_of_optimizations = if let Some(optimizer) = self.build.compiler.optimize {
            if optimizer {
                Some(self.build.compiler.optimizer_runs.unwrap_or(200))
            } else {
                None
            }
        } else {
            self.build.compiler.optimizer_runs
        };

        let verify = VerifyArgs {
            address,
            contract: Some(self.contract),
            compiler_version: Some(id.version.to_string()),
            constructor_args,
            constructor_args_path: None,
            num_of_optimizations,
            etherscan: EtherscanOpts { key: self.eth.etherscan.key(), chain: Some(chain.into()) },
            rpc: Default::default(),
            flatten: false,
            force: false,
            skip_is_verified_check: true,
            watch: true,
            retry: self.retry,
            libraries: self.build.libraries.clone(),
            root: None,
            verifier: self.verifier,
            via_ir: self.build.via_ir,
            evm_version: self.build.compiler.evm_version,
            show_standard_json_input: self.show_standard_json_input,
            guess_constructor_args: false,
            compilation_profile: Some(id.profile.to_string()),
        };
        sh_println!("Waiting for {} to detect contract deployment...", verify.verifier.verifier)?;
        verify.run().await
    }

    /// Parses the given constructor arguments into a vector of `DynSolValue`s, by matching them
    /// against the constructor's input params.
    ///
    /// Returns a list of parsed values that match the constructor's input params.
    fn parse_constructor_args(
        &self,
        constructor: &Constructor,
        constructor_args: &[String],
    ) -> Result<Vec<DynSolValue>> {
        let mut params = Vec::with_capacity(constructor.inputs.len());
        for (input, arg) in constructor.inputs.iter().zip(constructor_args) {
            // resolve the input type directly
            let ty = input
                .resolve()
                .wrap_err_with(|| format!("Could not resolve constructor arg: input={input}"))?;
            params.push((ty, arg));
        }
        let params = params.iter().map(|(ty, arg)| (ty, arg.as_str()));
        parse_tokens(params).map_err(Into::into)
    }
}

impl figment::Provider for CreateArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Create Args Provider")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut dict = Dict::default();
        if let Some(timeout) = self.timeout {
            dict.insert("transaction_timeout".to_string(), timeout.into());
        }
        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}

/// `ContractFactory` is a [`DeploymentTxFactory`] object with an
/// [`Arc`] middleware. This type alias exists to preserve backwards
/// compatibility with less-abstract Contracts.
///
/// For full usage docs, see [`DeploymentTxFactory`].
pub type ContractFactory<P> = DeploymentTxFactory<P>;

/// Helper which manages the deployment transaction of a smart contract. It
/// wraps a deployment transaction, and retrieves the contract address output
/// by it.
#[derive(Debug)]
#[must_use = "ContractDeploymentTx does nothing unless you `send` it"]
pub struct ContractDeploymentTx<P, C> {
    /// the actual deployer, exposed for overriding the defaults
    pub deployer: Deployer<P>,
    /// marker for the `Contract` type to create afterwards
    ///
    /// this type will be used to construct it via `From::from(Contract)`
    _contract: PhantomData<C>,
}

impl<P: Clone, C> Clone for ContractDeploymentTx<P, C> {
    fn clone(&self) -> Self {
        Self { deployer: self.deployer.clone(), _contract: self._contract }
    }
}

impl<P, C> From<Deployer<P>> for ContractDeploymentTx<P, C> {
    fn from(deployer: Deployer<P>) -> Self {
        Self { deployer, _contract: PhantomData }
    }
}

/// Helper which manages the deployment transaction of a smart contract
#[derive(Clone, Debug)]
#[must_use = "Deployer does nothing unless you `send` it"]
pub struct Deployer<P> {
    /// The deployer's transaction, exposed for overriding the defaults
    pub tx: WithOtherFields<TransactionRequest>,
    client: P,
    confs: usize,
    timeout: u64,
}

impl<P: Provider<AnyNetwork>> Deployer<P> {
    /// Broadcasts the contract deployment transaction and after waiting for it to
    /// be sufficiently confirmed (default: 1), it returns a tuple with the [`Address`] at the
    /// deployed contract's address and the corresponding [`AnyTransactionReceipt`].
    pub async fn send_with_receipt(
        self,
    ) -> Result<(Address, AnyTransactionReceipt), ContractDeploymentError> {
        let receipt = self
            .client
            .borrow()
            .send_transaction(self.tx)
            .await?
            .with_required_confirmations(self.confs as u64)
            .with_timeout(Some(Duration::from_secs(self.timeout)))
            .get_receipt()
            .await?;

        let address =
            receipt.contract_address.ok_or(ContractDeploymentError::ContractNotDeployed)?;

        Ok((address, receipt))
    }
}

/// To deploy a contract to the Ethereum network, a [`ContractFactory`] can be
/// created which manages the Contract bytecode and Application Binary Interface
/// (ABI), usually generated from the Solidity compiler.
#[derive(Clone, Debug)]
pub struct DeploymentTxFactory<P> {
    client: P,
    abi: JsonAbi,
    bytecode: Bytes,
    timeout: u64,
}

impl<P: Provider<AnyNetwork> + Clone> DeploymentTxFactory<P> {
    /// Creates a factory for deployment of the Contract with bytecode, and the
    /// constructor defined in the abi. The client will be used to send any deployment
    /// transaction.
    pub fn new(abi: JsonAbi, bytecode: Bytes, client: P, timeout: u64) -> Self {
        Self { client, abi, bytecode, timeout }
    }

    /// Create a deployment tx using the provided tokens as constructor
    /// arguments
    pub fn deploy_tokens(
        self,
        params: Vec<DynSolValue>,
    ) -> Result<Deployer<P>, ContractDeploymentError> {
        // Encode the constructor args & concatenate with the bytecode if necessary
        let data: Bytes = match (self.abi.constructor(), params.is_empty()) {
            (None, false) => return Err(ContractDeploymentError::ConstructorError),
            (None, true) => self.bytecode.clone(),
            (Some(constructor), _) => {
                let input: Bytes = constructor
                    .abi_encode_input(&params)
                    .map_err(ContractDeploymentError::DetokenizationError)?
                    .into();
                // Concatenate the bytecode and abi-encoded constructor call.
                self.bytecode.iter().copied().chain(input).collect()
            }
        };

        // create the tx object. Since we're deploying a contract, `to` is `None`
        let tx = WithOtherFields::new(TransactionRequest::default().input(data.into()));

        Ok(Deployer { client: self.client.clone(), tx, confs: 1, timeout: self.timeout })
    }
}

#[derive(thiserror::Error, Debug)]
/// An Error which is thrown when interacting with a smart contract
pub enum ContractDeploymentError {
    #[error("constructor is not defined in the ABI")]
    ConstructorError,
    #[error(transparent)]
    DetokenizationError(#[from] alloy_dyn_abi::Error),
    #[error("contract was not deployed")]
    ContractNotDeployed,
    #[error(transparent)]
    RpcError(#[from] TransportError),
}

impl From<PendingTransactionError> for ContractDeploymentError {
    fn from(_err: PendingTransactionError) -> Self {
        Self::ContractNotDeployed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::I256;

    #[test]
    fn can_parse_create() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--verify",
            "--retries",
            "10",
            "--delay",
            "30",
        ]);
        assert_eq!(args.retry.retries, 10);
        assert_eq!(args.retry.delay, 30);
    }
    #[test]
    fn can_parse_chain_id() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--verify",
            "--retries",
            "10",
            "--delay",
            "30",
            "--chain-id",
            "9999",
        ]);
        assert_eq!(args.chain_id(), Some(9999));
    }

    #[test]
    fn test_parse_constructor_args() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--constructor-args",
            "Hello",
        ]);
        let constructor: Constructor = serde_json::from_str(r#"{"type":"constructor","inputs":[{"name":"_name","type":"string","internalType":"string"}],"stateMutability":"nonpayable"}"#).unwrap();
        let params = args.parse_constructor_args(&constructor, &args.constructor_args).unwrap();
        assert_eq!(params, vec![DynSolValue::String("Hello".to_string())]);
    }

    #[test]
    fn test_parse_tuple_constructor_args() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--constructor-args",
            "[(1,2), (2,3), (3,4)]",
        ]);
        let constructor: Constructor = serde_json::from_str(r#"{"type":"constructor","inputs":[{"name":"_points","type":"tuple[]","internalType":"struct Point[]","components":[{"name":"x","type":"uint256","internalType":"uint256"},{"name":"y","type":"uint256","internalType":"uint256"}]}],"stateMutability":"nonpayable"}"#).unwrap();
        let _params = args.parse_constructor_args(&constructor, &args.constructor_args).unwrap();
    }

    #[test]
    fn test_parse_int_constructor_args() {
        let args: CreateArgs = CreateArgs::parse_from([
            "foundry-cli",
            "src/Domains.sol:Domains",
            "--constructor-args",
            "-5",
        ]);
        let constructor: Constructor = serde_json::from_str(r#"{"type":"constructor","inputs":[{"name":"_name","type":"int256","internalType":"int256"}],"stateMutability":"nonpayable"}"#).unwrap();
        let params = args.parse_constructor_args(&constructor, &args.constructor_args).unwrap();
        assert_eq!(params, vec![DynSolValue::Int(I256::unchecked_from(-5), 256)]);
    }
}
