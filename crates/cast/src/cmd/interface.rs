use alloy_json_abi::{ContractObject, InternalType, JsonAbi, Param, ToSolConfig};
use alloy_primitives::{
    Address,
    map::{HashMap, HashSet},
};
use clap::Parser;
use eyre::{Context, Result};
use forge_fmt::FormatterConfig;
use foundry_cli::{
    json::print_json_object,
    opts::EtherscanOpts,
    utils::{LoadConfig, fetch_abi_from_etherscan},
};
use foundry_common::{
    ContractsByArtifact,
    compile::{PathOrContractInfo, ProjectCompiler, compile_abi_project},
    find_target_path, fs, shell,
};
use foundry_config::load_config;
use itertools::Itertools;
use serde_json::Value;
use std::{path::PathBuf, str::FromStr};

/// CLI arguments for `cast interface`.
#[derive(Clone, Debug, Parser)]
pub struct InterfaceArgs {
    /// The target contract, which can be one of:
    /// - A file path to an ABI JSON file.
    /// - A contract identifier in the form `<path>:<contractname>` or just `<contractname>`.
    /// - An Ethereum address, for which the ABI will be fetched from Etherscan. If Etherscan
    ///   reports the contract as a proxy, the ABI of its implementation is included as well.
    contract: String,

    /// The name to use for the generated interface.
    ///
    /// Only relevant when retrieving the ABI from a file.
    #[arg(long, short)]
    name: Option<String>,

    /// Solidity pragma version.
    #[arg(long, short, default_value = "^0.8.4", value_name = "VERSION")]
    pragma: String,

    /// The path to the output file.
    ///
    /// If not specified, the interface will be output to stdout.
    #[arg(
        short,
        long,
        value_hint = clap::ValueHint::FilePath,
        value_name = "PATH",
    )]
    output: Option<PathBuf>,

    /// If set, generate all types in a single interface, inlining any inherited or library types.
    ///
    /// This can fail if there are structs with the same name in different interfaces.
    #[arg(long)]
    flatten: bool,

    #[command(flatten)]
    etherscan: EtherscanOpts,
}

impl InterfaceArgs {
    pub async fn run(self) -> Result<()> {
        let Self { contract, name, pragma, output, flatten, etherscan } = self;

        // The target is an ABI file, an Ethereum address, or a local contract.
        let is_json_file = fs::read_to_string(&contract)
            .is_ok_and(|content| serde_json::from_str::<Value>(&content).is_ok());
        let abis = if is_json_file {
            vec![(load_abi_from_file(&contract)?, name.unwrap_or_else(|| "Interface".to_owned()))]
        } else if let Ok(address) = Address::from_str(&contract) {
            fetch_abi_from_etherscan(address, &etherscan.load_config()?, true).await?
        } else {
            vec![load_abi_from_artifact(&contract)?]
        };

        let config = flatten.then(|| ToSolConfig::new().one_contract(true));
        let mut json_abis = Vec::with_capacity(abis.len());
        let mut sources = Vec::with_capacity(abis.len());
        let multiple = abis.len() > 1;
        let mut declarations = HashSet::default();
        for (mut abi, mut name) in abis {
            json_abis.push(serde_json::to_value(&abi)?);
            abi.dedup();
            if multiple {
                let mut names = HashMap::<_, _>::default();
                let unique = unique_declaration_name(&name, &mut declarations, Some(&abi));
                names.insert(name, unique.clone());
                name = unique;
                visit_abi_types(&mut abi, &mut |ty| {
                    if let InternalType::Struct { contract: Some(contract), .. }
                    | InternalType::Enum { contract: Some(contract), .. }
                    | InternalType::Other { contract: Some(contract), .. } = ty
                    {
                        *contract = names
                            .entry(contract.clone())
                            .or_insert_with(|| {
                                if flatten {
                                    contract.clone()
                                } else {
                                    unique_declaration_name(contract, &mut declarations, None)
                                }
                            })
                            .clone();
                    }
                });
            }
            let source = abi.to_sol(&name, config.clone());
            sources.push(
                match forge_fmt::format(&source, FormatterConfig::default()).into_result() {
                    Ok(formatted) => formatted,
                    Err(e) => {
                        sh_warn!("Failed to format interface for {name}: {e}")?;
                        source
                    }
                },
            );
        }
        let source = format!(
            "// SPDX-License-Identifier: UNLICENSED\n\
             pragma solidity {pragma};\n\n\
             {}",
            sources.iter().format("\n")
        );

        if let Some(loc) = output {
            let res =
                if shell::is_json() { serde_json::to_string_pretty(&json_abis)? } else { source };
            if let Some(parent) = loc.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&loc, res)?;
            sh_status!("Saved interface at {}", loc.display())?;
        } else if shell::is_json() {
            print_json_object(json_abis)?;
        } else {
            sh_print!("{source}")?;
        }
        Ok(())
    }
}

/// Reserves a declaration name across all generated interfaces and libraries.
fn unique_declaration_name(
    name: &str,
    declarations: &mut HashSet<String>,
    abi: Option<&JsonAbi>,
) -> String {
    let mut available = |candidate: &str| {
        !abi.is_some_and(|abi| abi.functions.contains_key(candidate))
            && declarations.insert(candidate.to_owned())
    };
    if available(name) {
        return name.to_owned();
    }
    for suffix in 1.. {
        let candidate = format!("{name}_{suffix}");
        if available(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Visits internal types, including nested tuples, in every ABI parameter.
fn visit_abi_types(abi: &mut JsonAbi, visit: &mut impl FnMut(&mut InternalType)) {
    if let Some(constructor) = abi.constructor_mut() {
        visit_param_types(&mut constructor.inputs, visit);
    }
    for function in abi.functions_mut() {
        visit_param_types(&mut function.inputs, visit);
        visit_param_types(&mut function.outputs, visit);
    }
    for error in abi.errors_mut() {
        visit_param_types(&mut error.inputs, visit);
    }
    for event in abi.events_mut() {
        for param in &mut event.inputs {
            if let Some(ty) = &mut param.internal_type {
                visit(ty);
            }
            visit_param_types(&mut param.components, visit);
        }
    }
}

fn visit_param_types(params: &mut [Param], visit: &mut impl FnMut(&mut InternalType)) {
    for param in params {
        if let Some(ty) = &mut param.internal_type {
            visit(ty);
        }
        visit_param_types(&mut param.components, visit);
    }
}

/// Load the ABI from a file.
pub(crate) fn load_abi_from_file(path: &str) -> Result<JsonAbi> {
    let file = std::fs::read_to_string(path).wrap_err("unable to read abi file")?;
    let obj: ContractObject = serde_json::from_str(&file)?;
    obj.abi.ok_or_else(|| eyre::eyre!("could not find ABI in file {path}"))
}

/// Load the ABI and name from the artifact of a locally compiled contract.
fn load_abi_from_artifact(path_or_contract: &str) -> Result<(JsonAbi, String)> {
    let config = load_config()?;
    let mut project = config.project()?;
    project.no_artifacts = true;
    let compiler = ProjectCompiler::new().quiet(true);

    let contract = PathOrContractInfo::from_str(path_or_contract)?;
    let target_path = find_target_path(&project, &contract)?;
    let output = compile_abi_project(&mut project, compiler.files([target_path.clone()]))?;

    let (abi, name) = ContractsByArtifact::from(output)
        .find_abi_by_name_or_src_path(contract.name().unwrap_or(&target_path.to_string_lossy()))
        .ok_or_else(|| eyre::eyre!("Failed to fetch lossless ABI"))?;
    Ok((abi, contract.name().unwrap_or(&name).to_string()))
}
