use crate::{cmd::forge::build, opts::cast::parse_slot, utils::try_consume_config_rpc_url};
use cast::Cast;
use clap::Parser;
use comfy_table::{presets::ASCII_MARKDOWN, Table};
use ethers::{
    abi::ethabi::ethereum_types::BigEndianHash, etherscan::Client, prelude::*,
    solc::artifacts::StorageLayout,
};
use eyre::{ContextCompat, Result};
use foundry_common::{
    abi::find_source,
    compile::{compile, etherscan_project, suppress_compile},
    try_get_http_provider, RetryProvider,
};
use foundry_config::Config;
use futures::future::join_all;
use semver::Version;
use std::str::FromStr;

/// The minimum Solc version for outputting storage layouts.
///
/// https://github.com/ethereum/solidity/blob/develop/Changelog.md#065-2020-04-06
const MIN_SOLC: Version = Version::new(0, 6, 5);

/// CLI arguments for `cast storage`.
#[derive(Debug, Clone, Parser)]
pub struct StorageArgs {
    // Storage
    #[clap(
        help = "The contract address.", 
        value_name = "ADDRESS",
        value_parser = NameOrAddress::from_str
    )]
    address: NameOrAddress,
    #[clap(
        help = "The storage slot number (hex or decimal)",
        value_parser = parse_slot,
        value_name = "SLOT"
    )]
    slot: Option<H256>,
    #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
    rpc_url: Option<String>,
    #[clap(
        long,
        short = 'B',
        help = "The block height you want to query at.",
        long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
        value_name = "BLOCK"
    )]
    block: Option<BlockId>,

    // Etherscan
    #[clap(long, short, env = "ETHERSCAN_API_KEY", help = "etherscan API key", value_name = "KEY")]
    etherscan_api_key: Option<String>,
    #[clap(
        long,
        visible_alias = "chain-id",
        env = "CHAIN",
        help = "The chain ID the contract is deployed to.",
        default_value = "mainnet",
        value_name = "CHAIN"
    )]
    chain: Chain,

    // Forge
    #[clap(flatten)]
    build: build::CoreBuildArgs,
}

impl StorageArgs {
    pub async fn run(self) -> Result<()> {
        let Self { address, block, build, rpc_url, slot, chain, etherscan_api_key } = self;

        let rpc_url = try_consume_config_rpc_url(rpc_url)?;
        let provider = try_get_http_provider(rpc_url)?;

        let address = match address {
            NameOrAddress::Name(name) => provider.resolve_name(&name).await?,
            NameOrAddress::Address(address) => address,
        };

        // Slot was provided, perform a simple RPC call
        if let Some(slot) = slot {
            let cast = Cast::new(provider);
            println!("{}", cast.storage(address, slot, block).await?);
            return Ok(())
        }

        // No slot was provided
        // Get deployed bytecode at given address
        let address_code = provider.get_code(address, block).await?;
        if address_code.is_empty() {
            eyre::bail!("Provided address has no deployed code and thus no storage");
        }

        // Check if we're in a forge project and if we can find the address' code
        let mut project = build.project()?;
        if project.paths.has_input_files() {
            // Find in artifacts and pretty print
            add_storage_layout_output(&mut project);
            let out = compile(&project, false, false)?;
            let match_code = |artifact: &ConfigurableContractArtifact| -> Option<bool> {
                let bytes =
                    artifact.deployed_bytecode.as_ref()?.bytecode.as_ref()?.object.as_bytes()?;
                Some(bytes == &address_code)
            };
            let artifact =
                out.artifacts().find(|(_, artifact)| match_code(artifact).unwrap_or_default());
            if let Some((_, artifact)) = artifact {
                return fetch_and_print_storage(provider, address, artifact, true).await
            }
        }

        // Not a forge project or artifact not found
        // Get code from Etherscan
        eprintln!("No matching artifacts found, fetching source code from Etherscan...");
        let api_key = etherscan_api_key.or_else(|| {
            let config = Config::load();
            config.get_etherscan_api_key(Some(chain))
        }).wrap_err("No Etherscan API Key is set. Consider using the ETHERSCAN_API_KEY env var, or setting the -e CLI argument or etherscan-api-key in foundry.toml")?;
        let client = Client::new(chain, api_key)?;
        let source = find_source(client, address).await?;
        let metadata = source.items.first().unwrap();
        if metadata.is_vyper() {
            eyre::bail!("Contract at provided address is not a valid Solidity contract")
        }

        let version = metadata.compiler_version()?;
        let auto_detect = version < MIN_SOLC;

        // Create a new temp project
        // TODO: Cache instead of using a temp directory: metadata from Etherscan won't change
        let root = tempfile::tempdir()?;
        let root_path = root.path();
        let mut project = etherscan_project(metadata, root_path)?;
        add_storage_layout_output(&mut project);
        project.auto_detect = auto_detect;

        // Compile
        let mut out = suppress_compile(&project)?;
        let artifact = {
            let (_, mut artifact) = out
                .artifacts()
                .find(|(name, _)| name == &metadata.contract_name)
                .ok_or_else(|| eyre::eyre!("Could not find artifact"))?;

            if is_storage_layout_empty(&artifact.storage_layout) && auto_detect {
                // try recompiling with the minimum version
                eprintln!("The requested contract was compiled with {version} while the minimum version for storage layouts is {MIN_SOLC} and as a result the output may be empty.");
                let solc = Solc::find_or_install_svm_version(MIN_SOLC.to_string())?;
                project.solc = solc;
                project.auto_detect = false;
                if let Ok(output) = suppress_compile(&project) {
                    out = output;
                    let (_, new_artifact) = out
                        .artifacts()
                        .find(|(name, _)| name == &metadata.contract_name)
                        .ok_or_else(|| eyre::eyre!("Could not find artifact"))?;
                    artifact = new_artifact;
                }
            }

            artifact
        };

        // Clear temp directory
        root.close()?;

        fetch_and_print_storage(provider, address, artifact, true).await
    }
}

async fn fetch_and_print_storage(
    provider: RetryProvider,
    address: Address,
    artifact: &ConfigurableContractArtifact,
    pretty: bool,
) -> Result<()> {
    if is_storage_layout_empty(&artifact.storage_layout) {
        eprintln!("Storage layout is empty.");
        Ok(())
    } else {
        let layout = artifact.storage_layout.as_ref().unwrap().clone();
        let values = fetch_storage_values(provider, address, &layout).await?;
        print_storage(layout, values, pretty)
    }
}

/// Overrides the `value` field in [StorageLayout] with the slot's value to avoid creating new data
/// structures.
async fn fetch_storage_values(
    provider: RetryProvider,
    address: Address,
    layout: &StorageLayout,
) -> Result<Vec<String>> {
    // TODO: Batch request; handle array values
    let futures: Vec<_> = layout
        .storage
        .iter()
        .map(|slot| {
            let slot_h256 = H256::from_uint(&U256::from_dec_str(&slot.slot)?);
            Ok(provider.get_storage_at(address, slot_h256, None))
        })
        .collect::<Result<_>>()?;

    // TODO: Better format values according to their Solidity type
    join_all(futures).await.into_iter().map(|value| Ok(format!("{}", value?.into_uint()))).collect()
}

fn print_storage(layout: StorageLayout, values: Vec<String>, pretty: bool) -> Result<()> {
    if !pretty {
        println!("{}", serde_json::to_string_pretty(&serde_json::to_value(layout)?)?);
        return Ok(())
    }

    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_header(vec!["Name", "Type", "Slot", "Offset", "Bytes", "Value", "Contract"]);

    for (slot, value) in layout.storage.into_iter().zip(values) {
        let storage_type = layout.types.get(&slot.storage_type);
        table.add_row(vec![
            slot.label,
            storage_type.as_ref().map_or("?".to_string(), |t| t.label.clone()),
            slot.slot,
            slot.offset.to_string(),
            storage_type.as_ref().map_or("?".to_string(), |t| t.number_of_bytes.clone()),
            value,
            slot.contract,
        ]);
    }

    println!("{table}");

    Ok(())
}

fn add_storage_layout_output(project: &mut Project) {
    project.artifacts.additional_values.storage_layout = true;
    let output_selection = project.artifacts.output_selection();
    project.solc_config.settings.push_all(output_selection);
}

fn is_storage_layout_empty(storage_layout: &Option<StorageLayout>) -> bool {
    if let Some(ref s) = storage_layout {
        s.storage.is_empty()
    } else {
        true
    }
}
