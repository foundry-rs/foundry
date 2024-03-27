use std::path::PathBuf;

use alloy_primitives::Address;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_block_explorers::{contract::Metadata, Client};
use foundry_cli::opts::EtherscanOpts;
use foundry_common::fs;
use foundry_compilers::artifacts::Settings;
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
        let init_args = InitArgs { root: root.clone(), vscode: true, ..Default::default() };
        init_args.run().map_err(|_| eyre::eyre!("Cannot run `clone` on a non-empty directory."))?;

        // canonicalize the root path
        // note that at this point, the root directory must have been created
        let root = dunce::canonicalize(root)?;

        // remove the unnecessary example contracts
        // XXX (ZZ): this is a temporary solution until we have a proper way to remove contracts,
        // e.g., add a field in the InitArgs to control the example contract generation
        fs::remove_file(root.join("src/Counter.sol"))?;
        fs::remove_file(root.join("test/Counter.t.sol"))?;
        fs::remove_file(root.join("script/Counter.s.sol"))?;

        // update configuration
        Config::update_at(root, |config, doc| {
            update_config_by_metadata(config, doc, &meta).is_ok()
        })?;

        Ok(())
    }
}

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
        libraries,
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
