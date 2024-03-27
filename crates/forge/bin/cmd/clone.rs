use std::{fs::read_dir, path::PathBuf};

use alloy_primitives::Address;
use clap::{Parser, ValueHint};
use eyre::{OptionExt, Result};
use foundry_block_explorers::{contract::Metadata, Client};
use foundry_cli::opts::EtherscanOpts;
use foundry_common::fs;
use foundry_compilers::artifacts::Settings;
use foundry_compilers::remappings::{RelativeRemapping, Remapping};
use foundry_config::Config;
use toml_edit;

use super::init::InitArgs;

/// CLI arguments for `forge clone`.
#[derive(Clone, Debug, Parser)]
pub struct CloneArgs {
    /// The contract address to clone.
    address: String,

    /// The root directory of the cloned project.
    #[arg(value_hint = ValueHint::DirPath, default_value = ".", value_name = "PATH")]
    root: PathBuf,

    #[command(flatten)]
    etherscan: EtherscanOpts,
}

impl CloneArgs {
    pub async fn run(self) -> Result<()> {
        let CloneArgs { address, root, etherscan } = self;

        // parse the contract address
        let contract_address: Address = address.parse()?;

        // get the chain and api key from the config
        let config = Config::from(&etherscan);
        let chain = config.chain.unwrap_or_default();
        let etherscan_api_key = config.get_etherscan_api_key(Some(chain)).unwrap_or_default();

        // get the contract code
        let client = Client::new(chain, etherscan_api_key)?;
        let mut meta = client.contract_source_code(contract_address).await?;
        if meta.items.len() != 1 {
            return Err(eyre::eyre!("contract not found or ill-formed"));
        }
        let meta = meta.items.remove(0);
        if meta.is_vyper() {
            return Err(eyre::eyre!("Vyper contracts are not supported"));
        }

        // let's try to init the project with default init args
        let init_args = InitArgs { root: root.clone(), ..Default::default() };
        init_args.run().map_err(|e| eyre::eyre!("Project init error: {:?}", e))?;

        // canonicalize the root path
        // note that at this point, the root directory must have been created
        let root = dunce::canonicalize(root)?;

        // remove the unnecessary example contracts
        // XXX (ZZ): this is a temporary solution until we have a proper way to remove contracts,
        // e.g., add a field in the InitArgs to control the example contract generation
        fs::remove_file(root.join("src/Counter.sol"))?;
        fs::remove_file(root.join("test/Counter.t.sol"))?;
        fs::remove_file(root.join("script/Counter.s.sol"))?;

        // dump sources and update the remapping in configuration
        let remappings = dump_sources(&meta, root.clone())?;
        Config::update_at(root.clone(), |config, doc| {
            let profile = config.profile.as_str().as_str();
            let mut remapping_array = toml_edit::Array::new();
            for r in remappings {
                remapping_array.push(r.to_string());
            }
            doc[Config::PROFILE_SECTION][profile]["remappings"] = toml_edit::value(remapping_array);

            true
        })?;

        // update configuration
        Config::update_at(root, |config, doc| {
            update_config_by_metadata(config, doc, &meta).is_ok()
        })?;

        Ok(())
    }
}

/// Update the configuration file with the metadata.
/// This function will update the configuration file with the metadata from the contract.
/// It will update the following fields:
/// - `auto_detect_solc` to `false`
/// - `solc_version` to the value from the metadata
/// - `evm_version` to the value from the metadata
/// - `via_ir` to the value from the metadata
/// - `libraries` to the value from the metadata
/// - `metadata` to the value from the metadata
///     - `cbor_metadata`, `use_literal_content`, and `bytecode_hash`
/// - `optimizer` to the value from the metadata
/// - `optimizer_runs` to the value from the metadata
/// - `optimizer_details` to the value from the metadata
///     - `yul_details`, `yul`, etc.
///     - `simpleCounterForLoopUncheckedIncrement` is ignored for now
/// - `remappings` and `stop_after` are pre-validated to be empty and None, respectively
/// - `model_checker`, `debug`, and `output_selection` are ignored for now
///
/// Detailed information can be found from the following link:
/// - https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options
/// - https://docs.soliditylang.org/en/latest/using-the-compiler.html#compiler-input-and-output-json-description
fn update_config_by_metadata(
    config: &Config,
    doc: &mut toml_edit::Document,
    meta: &Metadata,
) -> Result<()> {
    let profile = config.profile.as_str().as_str();

    // macro to update the config if the value exists
    macro_rules! update_if_needed {
        ([$($key:expr),+], $value:expr) => {
            {
                if let Some(value) = $value {
                    let mut current = &mut doc[Config::PROFILE_SECTION][profile];
                    $(
                        if let Some(nested_doc) = current.get_mut(&$key) {
                            current = nested_doc;
                        } else {
                            return Err(eyre::eyre!("cannot find the key: {}", $key));
                        }
                    )+
                    *current = toml_edit::value(value);
                }
            }
        };
    }

    // disable auto detect solc and set the solc version
    doc[Config::PROFILE_SECTION][profile]["auto_detect_solc"] = toml_edit::value(false);
    let version = meta.compiler_version()?;
    doc[Config::PROFILE_SECTION][profile]["solc_version"] =
        toml_edit::value(format!("{}.{}.{}", version.major, version.minor, version.patch));

    // get optimizer settings
    // XXX (ZZ): we ignore `model_checker`, `debug`, and `output_selection` for now,
    // it seems they do not have impacts on the actual compilation
    let Settings {
        optimizer,
        mut libraries,
        evm_version,
        via_ir,
        stop_after,
        remappings,
        metadata,
        ..
    } = meta.settings()?;
    eyre::ensure!(stop_after.is_none(), "stop_after should be None");
    eyre::ensure!(remappings.is_empty(), "remappings should be empty");

    update_if_needed!(["evm_version"], evm_version.map(|v| v.to_string()));
    update_if_needed!(["via_ir"], via_ir);

    // update metadata if needed
    if let Some(metadata) = metadata {
        update_if_needed!(["cbor_metadata"], metadata.cbor_metadata);
        update_if_needed!(["use_literal_content"], metadata.use_literal_content);
        update_if_needed!(["bytecode_hash"], metadata.bytecode_hash.map(|v| v.to_string()));
    }

    // update optimizer settings if needed
    update_if_needed!(["optimizer"], optimizer.enabled);
    update_if_needed!(["optimizer_runs"], optimizer.runs.map(|v| v as i64));
    // update optimizer details if needed
    if let Some(detail) = optimizer.details {
        doc[Config::PROFILE_SECTION][profile]["optimizer_details"] = toml_edit::table();

        update_if_needed!(["optimizer_details", "peephole"], detail.peephole);
        update_if_needed!(["optimizer_details", "inliner"], detail.inliner);
        update_if_needed!(["optimizer_details", "jumpdestRemover"], detail.jumpdest_remover);
        update_if_needed!(["optimizer_details", "orderLiterals"], detail.order_literals);
        update_if_needed!(["optimizer_details", "deduplicate"], detail.deduplicate);
        update_if_needed!(["optimizer_details", "cse"], detail.cse);
        update_if_needed!(["optimizer_details", "constantOptimizer"], detail.constant_optimizer);
        // XXX (ZZ): simpleCounterForLoopUncheckedIncrement seems not supported by fourndry
        update_if_needed!(["optimizer_details", "yul"], detail.yul);

        if let Some(yul_detail) = detail.yul_details {
            doc[Config::PROFILE_SECTION][profile]["optimizer_details"]["yulDetails"] =
                toml_edit::table();
            update_if_needed!(
                ["optimizer_details", "yulDetails", "stackAllocation"],
                yul_detail.stack_allocation
            );
            update_if_needed!(
                ["optimizer_details", "yulDetails", "optimizerSteps"],
                yul_detail.optimizer_steps
            );
        }
    }

    // apply remapping on libraries
    let path_config = config.project_paths();
    libraries = libraries.with_applied_remappings(&path_config);

    // update libraries
    let mut lib_array = toml_edit::Array::new();
    for (path_to_lib, info) in libraries.libs {
        for (lib_name, address) in info {
            lib_array.push(format!(
                "{}:{}:{}",
                path_to_lib.to_str().unwrap(),
                lib_name,
                address.to_string()
            ));
        }
    }
    doc[Config::PROFILE_SECTION][profile]["libraries"] = toml_edit::value(lib_array);

    Ok(())
}

/// Dump the contract sources to the root directory.
/// The sources are dumped to the `src` directory.
/// The library sources are dumped to the `lib` directory.
/// IO errors may be returned.
fn dump_sources(meta: &Metadata, root: PathBuf) -> Result<Vec<RelativeRemapping>> {
    // get config
    let config = Config::load_with_root(root.clone());

    let path_config = config.project_paths();
    let lib_dir = root
        .join(path_config.libraries.get(0).ok_or_eyre("no library path found")?)
        .canonicalize()?;
    let src_dir = root.join(path_config.sources).canonicalize()?;
    let contract_name = meta.contract_name.clone();
    let source_tree = meta.source_tree();

    // first we dump the sources to a temporary directory
    let tmp_dump_dir = root.join("raw_sources");
    source_tree
        .write_to(&tmp_dump_dir)
        .map_err(|e| eyre::eyre!("failed to dump sources: {}", e))?;

    // then we move the sources to the correct directories
    // 0. we will first load existing remappings if necessary
    //  make sure this happens before dumping sources
    let mut remappings: Vec<Remapping> = Remapping::find_many(lib_dir.clone());
    // we also load the original remappings from the metadata
    remappings.extend(meta.settings()?.remappings);

    // 1. move library sources to the `lib` directory (those with names starting with `@`)
    for entry in read_dir(tmp_dump_dir.join(contract_name.clone()))? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with("@") {
            if std::fs::metadata(&lib_dir).is_err() {
                std::fs::create_dir(&lib_dir)?;
            }
            let dest = lib_dir.join(entry.file_name());
            std::fs::rename(entry.path(), dest.clone())?;
            // add remapping entry
            remappings.push(Remapping {
                context: None,
                name: entry.file_name().to_string_lossy().to_string(),
                path: dest.to_string_lossy().to_string(),
            });
        }
    }
    // 2. move contract sources to the `src` directory
    for entry in std::fs::read_dir(tmp_dump_dir.join(contract_name))? {
        if std::fs::metadata(&src_dir).is_err() {
            std::fs::create_dir(&src_dir)?;
        }
        let entry = entry?;
        if entry.file_name().to_string_lossy().to_string().as_str() == "contracts" {
            // move all sub folders in contracts to src
            for e in read_dir(entry.path())? {
                let e = e?;
                let dest = src_dir.join(e.file_name());
                std::fs::rename(e.path(), dest.clone())?;
                remappings.push(Remapping {
                    context: None,
                    name: e.file_name().to_string_lossy().to_string(),
                    path: dest.to_string_lossy().to_string(),
                });
            }
        } else {
            // move the file to src
            let dest = src_dir.join(entry.file_name());
            std::fs::rename(entry.path(), dest.clone())?;
            remappings.push(Remapping {
                context: None,
                name: entry.file_name().to_string_lossy().to_string(),
                path: dest.to_string_lossy().to_string(),
            });
        }
    }

    // remove the temporary directory
    std::fs::remove_dir_all(tmp_dump_dir)?;

    Ok(remappings.into_iter().map(|r| r.into_relative(&root)).collect())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::CloneArgs;
    use foundry_common::compile::ProjectCompiler;
    use foundry_config::Config;
    use tempfile;

    fn assert_successful_compilation(root: &PathBuf) {
        println!("project_root: {:#?}", root);
        // change directory to the root
        std::env::set_current_dir(root).unwrap();
        let config = Config::load();
        let project = config.project().unwrap();
        let compiler = ProjectCompiler::new();
        let _ = compiler.compile(&project).expect("compilation failure");
    }

    #[tokio::test]
    async fn test_clone_single_file_contract() {
        let project_root = tempfile::tempdir().unwrap().path().to_path_buf();
        let args = CloneArgs {
            address: "0x35Fb958109b70799a8f9Bc2a8b1Ee4cC62034193".to_string(),
            root: project_root.clone(),
            etherscan: Default::default(),
        };
        args.run().await.unwrap();
        assert_successful_compilation(&project_root);
    }

    #[tokio::test]
    async fn test_clone_contract_with_optimization_details() {
        let project_root = tempfile::tempdir().unwrap().path().to_path_buf();
        let args = CloneArgs {
            address: "0x8B3D32cf2bb4d0D16656f4c0b04Fa546274f1545".to_string(),
            root: project_root.clone(),
            etherscan: Default::default(),
        };
        args.run().await.unwrap();
        assert_successful_compilation(&project_root);
    }
}
