use crate::{
    cmd::forge::{build, inspect::print_storage_layout},
    opts::cast::{parse_block_id, parse_name_or_address, parse_slot},
    utils::consume_config_rpc_url,
};
use cast::Cast;
use clap::Parser;
use contract::ContractMetadata;
use errors::EtherscanError;
use ethers::{
    etherscan::Client,
    prelude::*,
    solc::artifacts::{
        output_selection::ContractOutputSelection, BytecodeHash, Optimizer, Settings,
    },
};
use eyre::{ContextCompat, Result};
use foundry_common::{compile::compile, try_get_http_provider};
use foundry_config::Config;
use semver::Version;
use std::{future::Future, pin::Pin};

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
        let metadata = source.items.first().unwrap();

        let source_tree = source.source_tree()?;

        // Create a new temp project
        let root = tempfile::tempdir()?;
        let root_path = root.path();
        // let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/temp_build"));
        // let root_path = root.as_path();
        let sources = root_path.join(&metadata.contract_name);
        source_tree.write_to(root_path)?;

        // Configure Solc
        let paths = ProjectPathsConfig::builder().sources(sources).build_with_root(root_path);

        let mut settings = Settings::default();

        let mut optimizer = Optimizer::default();
        if metadata.optimization_used.trim().parse::<usize>()? == 1 {
            optimizer.enable();
            match metadata.runs.parse::<usize>() {
                Ok(runs) => optimizer.runs(runs),
                _ => {}
            };
        }
        settings.optimizer = optimizer;
        if !metadata.source_code.contains("pragma solidity") {
            eyre::bail!("Only Solidity verified contracts are allowed")
        }
        settings.evm_version = Some(metadata.evm_version.parse().unwrap_or_default());

        let version = metadata.compiler_version.as_str().trim();
        let solc = match version.strip_prefix('v').unwrap_or(version).parse::<Version>() {
            Ok(v) => {
                Solc::find_or_install_svm_version(&format!("{}.{}.{}", v.major, v.minor, v.patch))?
            }
            Err(_) => Solc::default(),
        }
        .with_base_path(root_path);
        let solc_config = SolcConfig::builder().settings(settings).build();

        let project = Project::builder()
            .solc(solc)
            .solc_config(solc_config)
            .no_auto_detect()
            .ephemeral()
            .no_artifacts()
            .ignore_error_code(1878) // License warning
            .ignore_error_code(5574) // Contract code size warning
            .paths(paths)
            .build()?;
        let mut project = with_storage_layout_output(project);

        // Compile
        let out = match compile(&project, false, false) {
            Ok(out) => Ok(out),
            // metadata does not contain many compiler settings...
            Err(e) => {
                if e.to_string().contains("--via-ir") {
                    println!(
                        "Compilation failed due to \"stack too deep\", retrying with \"--via-ir\"..."
                    );
                    project.solc_config.settings.via_ir = Some(true);
                    compile(&project, false, false)
                } else {
                    Err(e)
                }
            }
        }?;
        let artifact = out.artifacts().find(|(name, _)| name == &metadata.contract_name);
        let artifact = artifact.wrap_err("Artifact not found")?.1;

        print_storage_layout(&artifact.storage_layout, true)?;

        // Clear temp directory
        root.close()?;

        Ok(())
    }
}

fn with_storage_layout_output(mut project: Project) -> Project {
    project.solc_config.settings.metadata = Some(BytecodeHash::Ipfs.into());
    let settings = project.solc_config.settings.with_extra_output([
        ContractOutputSelection::Metadata,
        ContractOutputSelection::StorageLayout,
    ]);

    project.solc_config.settings = settings;
    project
}

/// If the code at `address` is a proxy, recurse until we find the implementation.
fn find_source(
    client: Client,
    address: Address,
) -> Pin<Box<dyn Future<Output = Result<ContractMetadata>>>> {
    Box::pin(async move {
        let source = client.contract_source_code(address).await?;
        let metadata = source.items.first().wrap_err("Etherscan returned no data")?;
        if metadata.proxy.parse::<usize>()? == 0 {
            Ok(source)
        } else {
            let implementation = metadata.implementation.parse()?;
            println!(
                "Contract at {} is a proxy, trying to fetch source at {:?}...",
                address, implementation
            );
            match find_source(client, implementation).await {
                impl_source @ Ok(_) => impl_source,
                Err(e) => {
                    let err = EtherscanError::ContractCodeNotVerified(address).to_string();
                    if e.to_string() == err {
                        println!("{}, using {}", err, address);
                        Ok(source)
                    } else {
                        Err(e)
                    }
                }
            }
        }
    })
}
