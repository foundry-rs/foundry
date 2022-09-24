use crate::{cast::parse_slot, cmd::Cmd, utils::consume_config_rpc_url};
use cast::Cast;
use clap::Parser;
use ethers::types::BlockId;
use eyre::Result;
use foundry_common::{parse_block_id, try_get_http_provider};

#[derive(Debug, Clone, Parser)]
pub struct StorageArgs {
    #[clap(help = "The contract address.", parse(try_from_str = parse_name_or_address), value_name = "ADDRESS")]
    address: NameOrAddress,
    #[clap(help = "The storage slot number (hex or decimal)", parse(try_from_str = parse_slot), value_name = "SLOT")]
    slot: Option<H256>,
    #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
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
}

impl StorageArgs {
    async fn run(&self) -> Result<()> {
        let rpc_url = consume_config_rpc_url(rpc_url);
        let provider = try_get_http_provider(rpc_url)?;
        let cast = Cast::new(provider);

        if let Some(slot) = self.slot {
            println!("{}", cast.storage(self.address, slot, self.block).await?);
            return Ok(())
        }

        Ok(())
    }
}
