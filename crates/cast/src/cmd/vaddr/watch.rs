use alloy_primitives::{Address, B256, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockNumberOrTag, Filter};
use eyre::Result;
use foundry_cli::{opts::RpcOpts, utils::LoadConfig};
use foundry_common::{provider::ProviderBuilder, shell};
use serde_json::json;
use std::sync::LazyLock;
use tempo_alloy::TempoNetwork;
use tempo_primitives::TempoAddressExt;

static TRANSFER_TOPIC: LazyLock<B256> =
    LazyLock::new(|| keccak256(b"Transfer(address,address,uint256)"));

pub(super) async fn run(
    addr: Address,
    token: Option<Address>,
    from_block: Option<u64>,
    rpc: RpcOpts,
) -> Result<()> {
    if !addr.is_virtual() {
        eyre::bail!("{addr} is not a virtual address");
    }

    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;

    // Transfer(address indexed from, address indexed to, uint256 value)
    // topic[0] = event sig, topic[1] = from, topic[2] = to
    let to_topic: B256 = {
        let mut buf = [0u8; 32];
        buf[12..].copy_from_slice(addr.as_slice());
        buf.into()
    };

    let start = from_block.map(BlockNumberOrTag::Number).unwrap_or(BlockNumberOrTag::Latest);

    let mut filter =
        Filter::new().event_signature(*TRANSFER_TOPIC).topic2(to_topic).from_block(start);

    if let Some(tok) = token {
        filter = filter.address(tok);
    }

    if !shell::is_json() {
        sh_println!("Watching transfers to {addr}... (Ctrl-C to stop)")?;
    }

    // Fetch logs from the requested start block (historical when from_block is set)
    let logs = provider.get_logs(&filter).await?;
    for log in &logs {
        print_transfer_log(log)?;
    }

    // Poll for new logs
    let mut last_block = provider.get_block_number().await?;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let current = provider.get_block_number().await?;
        if current > last_block {
            let poll_filter = filter.clone().from_block(last_block + 1).to_block(current);
            let new_logs = provider.get_logs(&poll_filter).await?;
            for log in &new_logs {
                print_transfer_log(log)?;
            }
            last_block = current;
        }
    }
}

fn print_transfer_log(log: &alloy_rpc_types::Log) -> Result<()> {
    let block = log.block_number.unwrap_or(0);
    let tx = log.transaction_hash.unwrap_or_default();
    let token = log.address();

    // Decode topics: topic[1]=from, topic[2]=to
    let from = log.topics().get(1).map(|t| {
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&t[12..]);
        Address::from(addr)
    });

    // Decode amount from data
    let amount = if log.data().data.len() >= 32 {
        alloy_primitives::U256::from_be_slice(&log.data().data[..32])
    } else {
        alloy_primitives::U256::ZERO
    };

    if shell::is_json() {
        sh_println!(
            "{}",
            serde_json::to_string(&json!({
                "block": block,
                "tx": format!("{tx}"),
                "token": format!("{token}"),
                "from": from.map(|a| format!("{a}")).unwrap_or_default(),
                "amount": amount.to_string(),
            }))?
        )?;
    } else {
        sh_println!(
            "block={block} tx={tx} token={token} from={} amount={amount}",
            from.map(|a| a.to_string()).unwrap_or_default(),
        )?;
    }
    Ok(())
}
