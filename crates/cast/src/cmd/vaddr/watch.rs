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
use tempo_alloy::TempoNetwork;
use tempo_primitives::TempoAddressExt;

trait LogSource {
    async fn get_logs(&self, filter: &Filter) -> Result<Vec<Log>>;
    async fn get_block_number(&self) -> Result<u64>;
}

impl<P: Provider<TempoNetwork>> LogSource for P {
    async fn get_logs(&self, filter: &Filter) -> Result<Vec<Log>> {
        Ok(Provider::get_logs(self, filter).await?)
    }

    async fn get_block_number(&self) -> Result<u64> {
        Ok(Provider::get_block_number(self).await?)
    }
}

async fn fetch_historical<S: LogSource>(source: &S, filter: &Filter) -> Result<(Vec<Log>, u64)> {
    let anchor = source.get_block_number().await?;
    // Resolve a latest start against the same anchor as the historical upper bound.
    let start = filter.get_from_block().unwrap_or(anchor);
    let filter = filter.clone().from_block(start).to_block(anchor);
    let logs = source.get_logs(&filter).await?;
    Ok((logs, anchor))
}

async fn poll_once<S: LogSource>(
    source: &S,
    filter: &Filter,
    last_block: u64,
) -> Result<Option<(Vec<Log>, u64)>> {
    let current = source.get_block_number().await?;
    if current > last_block {
        let filter = filter.clone().from_block(last_block + 1).to_block(current);
        let logs = source.get_logs(&filter).await?;
        Ok(Some((logs, current)))
    } else {
        Ok(None)
    }
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
    let (logs, mut last_block) = fetch_historical(&provider, &filter).await?;
    for log in logs {
        print_transfer_log(&log)?;
    }
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if let Some((logs, current)) = poll_once(&provider, &filter, last_block).await? {
            for log in logs {
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
    use std::cell::{Cell, RefCell};

    struct FakeLogSource {
        logs: Vec<Log>,
        tip: Cell<u64>,
        calls: Cell<usize>,
        advance_before_call: usize,
        advance_by: u64,
        ranges: RefCell<Vec<(u64, u64)>>,
    }

    impl FakeLogSource {
        fn before_call(&self) {
            let call = self.calls.get() + 1;
            self.calls.set(call);
            if call == self.advance_before_call {
                self.tip.set(self.tip.get() + self.advance_by);
            }
        }
    }

    impl LogSource for FakeLogSource {
        async fn get_logs(&self, filter: &Filter) -> Result<Vec<Log>> {
            self.before_call();
            let start = filter.get_from_block().unwrap_or(self.tip.get());
            let end = filter.get_to_block().unwrap_or(self.tip.get());
            self.ranges.borrow_mut().push((start, end));
            Ok(self
                .logs
                .iter()
                .filter(|log| {
                    (start..=end.min(self.tip.get())).contains(&log.block_number.unwrap())
                })
                .cloned()
                .collect())
        }

        async fn get_block_number(&self) -> Result<u64> {
            self.before_call();
            Ok(self.tip.get())
        }
    }

    #[tokio::test]
    async fn history_and_poll_share_anchor_when_tip_advances() {
        let addr = Address::repeat_byte(0x42);
        let token = Address::repeat_byte(0x11);
        // Exercise both explicit history and the default latest start.
        for start in [BlockNumberOrTag::Number(0), BlockNumberOrTag::Latest] {
            let source = FakeLogSource {
                logs: [100, 102, 104, 106]
                    .into_iter()
                    .map(|block| Log {
                        inner: alloy_primitives::Log::new_unchecked(
                            token,
                            vec![
                                IERC20::Transfer::SIGNATURE_HASH,
                                Address::ZERO.into_word(),
                                addr.into_word(),
                            ],
                            U256::from(1).to_be_bytes::<32>().into(),
                        ),
                        block_number: Some(block),
                        ..Default::default()
                    })
                    .collect(),
                tip: Cell::new(100),
                calls: Cell::new(0),
                advance_before_call: 2,
                advance_by: 6,
                ranges: RefCell::new(Vec::new()),
            };
            let filter = Filter::new()
                .event_signature(IERC20::Transfer::SIGNATURE_HASH)
                .topic2(addr.into_word())
                .address(token)
                .from_block(start);

            let (logs, anchor) = fetch_historical(&source, &filter).await.unwrap();
            assert_eq!(logs.iter().map(|log| log.block_number.unwrap()).collect::<Vec<_>>(), [100]);
            assert_eq!(anchor, 100);
            assert_eq!(source.calls.get(), 2);

            let (logs, current) = poll_once(&source, &filter, anchor).await.unwrap().unwrap();
            assert_eq!(current, 106);
            assert_eq!(
                logs.iter().map(|log| log.block_number.unwrap()).collect::<Vec<_>>(),
                [102, 104, 106]
            );
            assert_eq!(
                *source.ranges.borrow(),
                [(start.as_number().unwrap_or(100), 100), (101, 106)]
            );

            // Independently resolving the poll anchor after history would skip the entire gap.
            let old_history = 0..=100;
            let old_poll = 107..;
            assert!(
                (101..=106)
                    .all(|block| !old_history.contains(&block) && !old_poll.contains(&block))
            );
            assert!((101..=106).all(|block| {
                source.ranges.borrow().iter().any(|&(from, to)| (from..=to).contains(&block))
            }));

            assert!(poll_once(&source, &filter, current).await.unwrap().is_none());
            assert_eq!(source.calls.get(), 5);
            assert_eq!(source.ranges.borrow().len(), 2);
        }
    }
}
