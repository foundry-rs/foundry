use crate::{cmd::erc20::IERC20, tempo::tempo_provider};
use alloy_primitives::{Address, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, Filter, Log};
use alloy_sol_types::SolEvent;
use eyre::Result;
use foundry_cli::opts::RpcOpts;
use foundry_common::shell;
use serde_json::json;
use std::time::Duration;
use tempo_primitives::TempoAddressExt;

fn historical_filter(filter: &Filter, anchor: u64) -> Filter {
    filter.clone().from_block(filter.get_from_block().unwrap_or(anchor)).to_block(anchor)
}

fn poll_filter(filter: &Filter, last_block: u64, current: u64) -> Option<Filter> {
    (current > last_block).then(|| filter.clone().from_block(last_block + 1).to_block(current))
}

pub(super) async fn run(
    addr: Address,
    token: Option<Address>,
    from_block: Option<u64>,
    rpc: RpcOpts,
) -> Result<()> {
    if !addr.is_virtual() {
        eyre::bail!("{addr} is not a virtual address");
    }

    let (_, provider) = tempo_provider(&rpc)?;

    // Transfer(address indexed from, address indexed to, uint256 value): topic2 is the recipient.
    let start = from_block.map_or(BlockNumberOrTag::Latest, BlockNumberOrTag::Number);
    let mut filter = Filter::new()
        .event_signature(IERC20::Transfer::SIGNATURE_HASH)
        .topic2(addr.into_word())
        .from_block(start);
    if let Some(token) = token {
        filter = filter.address(token);
    }

    if !shell::is_json() {
        sh_status!("Watching transfers to {addr}... (Ctrl-C to stop)")?;
    }

    // Historical logs from the requested start block, then poll for new ones.
    let mut last_block = provider.get_block_number().await?;
    for log in provider.get_logs(&historical_filter(&filter, last_block)).await? {
        print_transfer_log(&log)?;
    }
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let current = provider.get_block_number().await?;
        if let Some(filter) = poll_filter(&filter, last_block, current) {
            for log in provider.get_logs(&filter).await? {
                print_transfer_log(&log)?;
            }
            last_block = current;
        }
    }
}

fn print_transfer_log(log: &Log) -> Result<()> {
    let block = log.block_number.unwrap_or(0);
    let tx = log.transaction_hash.unwrap_or_default();
    let token = log.address();
    let from = log.topics().get(1).map(|t| Address::from_word(*t).to_string()).unwrap_or_default();
    let data = &log.data().data;
    let amount = if data.len() >= 32 { U256::from_be_slice(&data[..32]) } else { U256::ZERO };

    if shell::is_json() {
        let payload = json!({
            "block": block,
            "tx": format!("{tx}"),
            "token": format!("{token}"),
            "from": from,
            "amount": amount.to_string(),
        });
        sh_println!("{payload}")
    } else {
        sh_println!("block={block} tx={tx} token={token} from={from} amount={amount}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_and_poll_ranges_are_contiguous() {
        for start in [BlockNumberOrTag::Number(0), BlockNumberOrTag::Latest] {
            let filter = Filter::new().from_block(start);
            let historical = historical_filter(&filter, 100);
            let poll = poll_filter(&filter, 100, 106).unwrap();

            assert_eq!(historical.get_from_block(), start.as_number().or(Some(100)));
            assert_eq!(historical.get_to_block(), Some(100));
            assert_eq!(poll.get_from_block(), Some(101));
            assert_eq!(poll.get_to_block(), Some(106));
        }

        assert!(poll_filter(&Filter::new(), 100, 100).is_none());
    }
}
