use crate::{
    cmd::forge::{build, inspect::print_storage_layout},
    opts::cast::{parse_block_id, parse_name_or_address, parse_slot},
    utils::consume_config_rpc_url,
};
use cast::Cast;
use clap::Parser;
use ethers::{etherscan::Client, prelude::*};
use eyre::{ContextCompat, Result};
use foundry_common::{
    compile::{compile, etherscan_project, suppress_compile},
    try_get_http_provider,
};
use foundry_config::Config;
use foundry_utils::find_source;
use semver::Version;

/// The minimum Solc version for outputting storage layouts.
///
/// https://github.com/ethereum/solidity/blob/develop/Changelog.md#065-2020-04-06
const MIN_SOLC: Version = Version::new(0, 6, 5);

#[derive(Debug, Clone, Parser)]
pub struct StorageArgs {
    // Storage
    #[clap(help = "The contract address.", parse(try_from_str = parse_name_or_address), value_name = "ADDRESS")]
    address: NameOrAddress,
    #[clap(help = "The storage slot number (hex or decimal)", parse(try_from_str = parse_slot), value_name = "SLOT")]
    slot: Option<H256>,
    #[clap(long, env = "ETH_RPC_URL", value_name = "URL")]
    rpc_url: Option<String>,
    #[clap(
        long,
        short = 'B',
        help = "The block height you want to query at.",
        long_help = "The block height you want to query at. Can also be the tags earliest, latest, or pending.",
        parse(try_from_str = parse_block_id),
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
        let StorageArgs { address, block, build, rpc_url, slot, chain, etherscan_api_key } = self;

        let rpc_url = consume_config_rpc_url(rpc_url);
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

        // Check if we're in a forge project
        let project = build.project()?;
        if project.paths.has_input_files() {
            // Find in artifacts and pretty print
            let project = with_storage_layout_output(project);
            let out = compile(&project, false, false)?;
            let match_code = |artifact: &ConfigurableContractArtifact| -> Option<bool> {
                let bytes =
                    artifact.deployed_bytecode.as_ref()?.bytecode.as_ref()?.object.as_bytes()?;
                Some(bytes == &address_code)
            };
            let artifact =
                out.artifacts().find(|(_, artifact)| match_code(artifact).unwrap_or_default());
            if let Some((_, artifact)) = artifact {
                return print_storage_layout(&artifact.storage_layout, true)
            }
        }

        // Not a forge project or artifact not found
        // Get code from Etherscan
        println!("No artifacts found, fetching source code from Etherscan...");
        let api_key = etherscan_api_key.or_else(|| {
            let config = Config::load();
            config.get_etherscan_api_key(Some(chain))
        }).ok_or_else(|| eyre::eyre!("No Etherscan API Key is set. Consider using the ETHERSCAN_API_KEY env var, or setting the -e CLI argument or etherscan-api-key in foundry.toml"))?;
        let client = Client::new(chain, api_key)?;
        let source = find_source(client, address).await?;
        let metadata = source.items.first().wrap_err("etherscan returned empty metadata")?;
        if !metadata.is_vyper() {
            eyre::bail!("Contract at provided address is not a valid Solidity contract")
        }

        let version = metadata.compiler_version()?;
        let auto_detect = version < MIN_SOLC;
        if auto_detect {
            println!("The requested contract was compiled with {} while the minimum version for storage layouts is {} and as a result it may be empty.", version, MIN_SOLC);
        }

        let root = tempfile::tempdir()?;
        let root_path = root.path();
        let project = etherscan_project(&metadata, root_path)?;
        let mut project = with_storage_layout_output(project);
        project.auto_detect = auto_detect;

        // Compile
        let out = suppress_compile(&project)?;
        dbg!(out.artifacts().count());
        let artifact = out.artifacts().find(|(name, _)| {
            println!("Artifact: {}", name);
            name == &metadata.contract_name
        });
        let artifact = artifact.wrap_err("Artifact not found")?.1;

        print_storage_layout(&artifact.storage_layout, true)?;

        // Clear temp directory
        root.close()?;

        Ok(())
    }
}

fn with_storage_layout_output(mut project: Project) -> Project {
    project.artifacts.additional_values.storage_layout = true;
    let output_selection = project.artifacts.output_selection();
    let settings = project.solc_config.settings.with_extra_output(output_selection);
    project.solc_config.settings = settings;
    project
}
