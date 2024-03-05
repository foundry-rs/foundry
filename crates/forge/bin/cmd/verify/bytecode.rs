use crate::cmd::build::BuildArgs;
use alloy_primitives::Address;
use alloy_rpc_types::{BlockId, BlockNumberOrTag};
use clap::{Parser, ValueHint};
use ethers_providers::Middleware;
use eyre::{OptionExt, Result};
use forge::constants::DEFAULT_CREATE2_DEPLOYER;
use foundry_block_explorers::Client;
use foundry_cli::{
    opts::EtherscanOpts,
    utils::{self, LoadConfig},
};
use foundry_common::types::ToEthers;
use foundry_config::{figment, impl_figment_convert, Config};
use std::path::PathBuf;
/// CLI arguments for `forge verify-bytecode`.
#[derive(Clone, Debug, Parser)]
pub struct VerifyBytecodeArgs {
    /// The address of the contract to verify.
    pub address: Address,

    /// The block at which the bytecode should be verified.
    #[clap(long, value_name = "BLOCK")]
    pub block: Option<BlockId>,

    /// The constructor args to generate the creation code.
    #[clap(
        long,
        conflicts_with = "constructor_args_path",
        value_name = "ARGS",
        visible_alias = "encoded-constructor-args"
    )]
    pub constructor_args: Option<String>,

    /// The path to a file containing the constructor arguments.
    #[clap(long, value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub constructor_args_path: Option<PathBuf>,

    /// Try to extract constructor arguments from on-chain creation code.
    #[arg(long)]
    pub guess_constructor_args: bool,

    /// The rpc url to use for verification.
    #[clap(long, value_name = "RPC_URL")]
    pub rpc_url: Option<String>,

    /// Verfication Type: `full` or `partial`. Ref: https://docs.sourcify.dev/docs/full-vs-partial-match/
    #[clap(long, default_value = "full", value_name = "TYPE")]
    pub verification_type: String,

    /// BuildArgs to use for the verification.
    #[clap(flatten)]
    pub build_opts: BuildArgs,

    pub root: Option<PathBuf>,

    #[clap(flatten)]
    pub etherscan_opts: EtherscanOpts,
}

impl_figment_convert!(VerifyBytecodeArgs);

impl figment::Provider for VerifyBytecodeArgs {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("Verify Bytecode Provider")
    }

    fn data(
        &self,
    ) -> Result<figment::value::Map<figment::Profile, figment::value::Dict>, figment::Error> {
        let mut dict = figment::value::Dict::new();
        if let Some(block) = &self.block {
            dict.insert("block".into(), figment::value::Value::serialize(block)?);
        }
        if let Some(rpc_url) = &self.rpc_url {
            dict.insert("rpc_url".into(), rpc_url.to_string().into());
        }
        dict.insert("verification_type".into(), self.verification_type.to_string().into());

        // if let Some(root) = self.root.as_ref() {
        //     dict.insert("root".to_string(), figment::value::Value::serialize(root)?);
        // }
        Ok(figment::value::Map::from([(Config::selected_profile(), dict)]))
    }
}

impl VerifyBytecodeArgs {
    /// Run the `verify-bytecode` command to verify the bytecode onchain against the locally built
    /// bytecode.
    pub async fn run(mut self) -> Result<()> {
        let config = self.load_config_emit_warnings();
        let provider = utils::get_provider(&config)?;

        // If chain is not set, we try to get it from the RPC
        // If RPC is not set, the default chain is used

        let chain = match config.get_rpc_url() {
            Some(_) => utils::get_chain(config.chain, provider.clone()).await?,
            None => config.chain.unwrap_or_default(),
        };

        // Set Etherscan options
        self.etherscan_opts.chain = Some(chain);
        self.etherscan_opts.key =
            config.get_etherscan_config_with_chain(Some(chain))?.map(|c| c.key);
        // Create etherscan client
        let etherscan = Client::new(chain, self.etherscan_opts.key.unwrap())?;

        // Get the constructor args using `source_code` endpoint
        let source_code = etherscan.contract_source_code(self.address).await?;

        let _constructor_args = match source_code.items.get(0) {
            Some(item) => item.constructor_arguments.clone(),
            None => {
                eyre::bail!("No source code found for contract at address {}", self.address);
            }
        };

        // Get creation tx hash
        let creation_data = etherscan.contract_creation_data(self.address).await?;
        let transaction = provider
            .get_transaction(creation_data.transaction_hash.to_ethers())
            .await?
            .ok_or_eyre("Couldn't fetch transaction data from RPC")?;
        let receipt = provider
            .get_transaction_receipt(creation_data.transaction_hash.to_ethers())
            .await?
            .ok_or_eyre("Couldn't fetch transaction receipt from RPC")?;

        // Extract creation code
        let _maybe_creation_code = if receipt.contract_address == Some(self.address.to_ethers()) {
            &transaction.input
        } else if transaction.to == Some(DEFAULT_CREATE2_DEPLOYER.to_ethers()) {
            &transaction.input[32..]
        } else {
            eyre::bail!(
                "Could not extract the creation code for contract at address {}",
                self.address
            );
        };

        // Get the block number of the creation tx
        let _simulation_block = match self.block {
            Some(block) => block,
            None => {
                let provider = utils::get_provider(&config)?;
                let creation_block =
                    provider.get_transaction(creation_data.transaction_hash.to_ethers()).await?;
                let block = match creation_block {
                    Some(tx) => tx.block_number.unwrap().as_u64(),
                    None => {
                        eyre::bail!(
                            "Failed to get block number of the creation tx, specify using
        the --block flag"
                        );
                    }
                };

                BlockId::Number(BlockNumberOrTag::Number(block))
            }
        };

        // TODO: @Yash
        // Compile the project
        // Cmp creation code with locally and built and maybe_creation_code
        // Fork the chain at `simulation_block`, deploy the contract and compare the runtime
        // bytecode.
        Ok(())
    }
}
