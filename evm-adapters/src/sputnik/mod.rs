mod evm;
pub use evm::*;

mod forked_backend;
pub use forked_backend::ForkMemoryBackend;

use ethers::providers::Middleware;
use sputnik::backend::MemoryVicinity;

pub async fn vicinity<M: Middleware>(
    provider: &M,
    pin_block: Option<u64>,
) -> Result<MemoryVicinity, M::Error> {
    let block_number = if let Some(pin_block) = pin_block {
        pin_block
    } else {
        provider.get_block_number().await?.as_u64()
    };

    let gas_price = provider.get_gas_price().await?;
    let chain_id = provider.get_chainid().await?;
    let block = provider.get_block(block_number).await?.expect("block not found");

    Ok(MemoryVicinity {
        origin: Default::default(),
        chain_id,
        block_hashes: Vec::new(),
        block_number: block.number.expect("block number not found").as_u64().into(),
        block_coinbase: block.author,
        block_difficulty: block.difficulty,
        block_gas_limit: block.gas_limit,
        block_timestamp: block.timestamp,
        gas_price,
    })
}
