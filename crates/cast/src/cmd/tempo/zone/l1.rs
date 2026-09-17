//! Parent-chain selection and correlated withdrawal settlement.

use super::abi::{IZoneOutbox, IZonePortal, OUTBOX};
use alloy_primitives::{Address, B256, keccak256};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types::Filter;
use alloy_sol_types::SolEvent;
use clap::Parser;
use eyre::{Result, ensure, eyre};
use std::time::Duration;
use tempo_alloy::TempoNetwork;

/// L1 connection options, deliberately separate from the authenticated zone RPC.
#[derive(Debug, Parser)]
pub(super) struct L1Args {
    /// Override the public mainnet or Moderato RPC inferred from the zone chain ID.
    #[arg(long, env = "L1_RPC_URL")]
    l1_rpc_url: Option<String>,
    /// Zone portal on Tempo L1. Required for --wait-l1 and Earn callback builders.
    #[arg(long, env = "L1_PORTAL_ADDRESS")]
    portal: Option<Address>,
}

impl L1Args {
    pub(super) fn portal(&self) -> Result<Address> {
        self.portal.ok_or_else(|| eyre!("--portal is required for L1 operations"))
    }

    pub(super) async fn provider(
        &self,
        zone_chain_id: u64,
        zone_id: u32,
    ) -> Result<RootProvider<TempoNetwork>> {
        let portal = self.portal()?;
        let parent = parent_chain(zone_chain_id, zone_id)?;
        let url = match self.l1_rpc_url.as_deref() {
            Some(url) => url,
            None => public_rpc(parent)?,
        };
        let provider = RootProvider::<TempoNetwork>::new_http(url.parse()?);
        ensure!(
            provider.get_chain_id().await? == parent,
            "L1 RPC chain ID does not match the zone's parent chain"
        );
        ensure!(
            IZonePortal::new(portal, &provider).zoneId().call().await? == zone_id,
            "--portal does not belong to --zone-id"
        );
        Ok(provider)
    }
}

// Chain ID allocation mirrored from zones/crates/primitives/src/constants.rs at a1c15e9f.
fn parent_chain(chain: u64, zone: u32) -> Result<u64> {
    ensure!(zone != 0, "--zone-id must be nonzero");
    let (parent, decoded_zone) = match chain {
        421_700_000..1_424_310_000 => (4217, chain - 421_700_000),
        1_424_310_000..2_147_483_648 => (42431, chain - 1_424_310_000),
        _ if chain >= 1 << 32 => (chain >> 32, chain & 0xffff_ffff),
        _ => return Err(eyre!("invalid zone chain ID: {chain}")),
    };
    ensure!(decoded_zone == u64::from(zone), "--zone-id does not match --zone-chain-id");
    ensure!(parent <= (1 << 20) - 2, "invalid parent chain ID");
    ensure!(chain < 1 << 32 || !matches!(parent, 4217 | 42431), "noncanonical zone chain ID");
    Ok(parent)
}

fn public_rpc(parent: u64) -> Result<&'static str> {
    match parent {
        4217 => Ok("https://rpc.tempo.xyz"),
        42431 => Ok("https://rpc.moderato.tempo.xyz"),
        _ => Err(eyre!("no public RPC for parent chain {parent}; supply --l1-rpc-url")),
    }
}

fn sender_tag(sender: Address, tx_hash: B256, nonce: u64) -> B256 {
    let mut bytes = [0u8; 60];
    bytes[..20].copy_from_slice(sender.as_slice());
    bytes[20..52].copy_from_slice(tx_hash.as_slice());
    bytes[52..].copy_from_slice(&nonce.to_be_bytes());
    keccak256(bytes)
}

pub(super) async fn wait_for_withdrawal(
    zone: &impl Provider<TempoNetwork>,
    l1: &impl Provider<TempoNetwork>,
    portal: Address,
    mut from_block: u64,
    zone_block: u64,
    zone_hash: B256,
) -> Result<B256> {
    let logs = zone
        .get_logs(
            &Filter::new()
                .address(OUTBOX)
                .event_signature(IZoneOutbox::WithdrawalRequested::SIGNATURE_HASH)
                .from_block(zone_block)
                .to_block(zone_block),
        )
        .await?;
    let requested = logs
        .iter()
        .filter(|log| log.transaction_hash == Some(zone_hash))
        .find_map(|log| IZoneOutbox::WithdrawalRequested::decode_log(&log.inner).ok())
        .ok_or_else(|| eyre!("withdrawal request event missing from {zone_hash}"))?;
    let tag = sender_tag(requested.sender, zone_hash, requested.fallbackNonce);
    loop {
        let head = l1.get_block_number().await?;
        if head >= from_block {
            // Bound each query for public RPC providers and advance through historical ranges.
            let end = head.min(from_block.saturating_add(999));
            let logs = l1
                .get_logs(
                    &Filter::new()
                        .address(portal)
                        .event_signature(IZonePortal::WithdrawalProcessed::SIGNATURE_HASH)
                        .topic1(requested.to.into_word())
                        .topic2(tag)
                        .from_block(from_block)
                        .to_block(end),
                )
                .await?;
            for log in logs {
                let event = IZonePortal::WithdrawalProcessed::decode_log(&log.inner)?;
                if !log.removed
                    && event.senderTag == tag
                    && event.to == requested.to
                    && event.token == requested.token
                    && event.amount == requested.amount
                {
                    let hash = log
                        .transaction_hash
                        .ok_or_else(|| eyre!("L1 event missing transaction hash"))?;
                    ensure!(
                        event.callbackSuccess,
                        "L1 delivery failed in {hash}; a refund was queued for the zone fallback recipient"
                    );
                    return Ok(hash);
                }
            }
            from_block = end + 1;
            if end < head {
                continue;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use alloy_provider::{ProviderBuilder, mock::Asserter};
    use alloy_rpc_types::Log;

    #[test]
    fn parent_chain_and_public_defaults() {
        assert_eq!(
            public_rpc(parent_chain(421_700_007, 7).unwrap()).unwrap(),
            "https://rpc.tempo.xyz"
        );
        assert_eq!(
            public_rpc(parent_chain(1_424_310_003, 3).unwrap()).unwrap(),
            "https://rpc.moderato.tempo.xyz"
        );
        assert_eq!(parent_chain((31337 << 32) | 2, 2).unwrap(), 31337);
        assert!(public_rpc(31337).is_err());
        for (chain, zone) in
            [(421_700_007, 8), (1_424_310_000, 0), (1 << 31, 1), ((4217 << 32) | 1, 1)]
        {
            assert!(parent_chain(chain, zone).is_err());
        }
    }

    #[test]
    fn sender_tag_binds_transaction_and_nonce() {
        let sender = Address::repeat_byte(0x11);
        let hash = B256::repeat_byte(0x22);
        let expected =
            keccak256([sender.as_slice(), hash.as_slice(), &3u64.to_be_bytes()].concat());
        assert_eq!(sender_tag(sender, hash, 3), expected);
        assert_ne!(sender_tag(sender, hash, 3), sender_tag(sender, hash, 4));
        assert_ne!(sender_tag(sender, hash, 3), sender_tag(sender, B256::ZERO, 3));
    }
    #[tokio::test]
    async fn wait_checks_delivery_result_and_ignores_other_transactions() {
        for success in [Some(true), Some(false), None] {
            let zone_hash = B256::repeat_byte(1);
            let l1_hash = B256::repeat_byte(2);
            let portal = Address::repeat_byte(3);
            let request = IZoneOutbox::WithdrawalRequested {
                withdrawalIndex: 0,
                sender: Address::repeat_byte(4),
                token: Address::repeat_byte(5),
                to: Address::repeat_byte(6),
                amount: 7,
                fee: 0,
                memo: B256::ZERO,
                gasLimit: 100_000,
                fallbackNonce: 8,
                data: Bytes::new(),
                revealTo: Bytes::new(),
            };
            let request_log = Log {
                inner: alloy_primitives::Log { address: OUTBOX, data: request.encode_log_data() },
                transaction_hash: Some(zone_hash),
                ..Default::default()
            };
            let mut unrelated = request_log.clone();
            unrelated.transaction_hash = Some(B256::ZERO);
            let zone_responses = Asserter::new();
            zone_responses.push_success(&vec![unrelated, request_log]);
            let zone = ProviderBuilder::new()
                .network::<TempoNetwork>()
                .connect_mocked_client(zone_responses);
            let delivered = IZonePortal::WithdrawalProcessed {
                to: request.to,
                senderTag: sender_tag(request.sender, zone_hash, 8),
                token: request.token,
                amount: request.amount,
                callbackSuccess: success.unwrap_or(true),
            };
            let delivered_log = Log {
                inner: alloy_primitives::Log { address: portal, data: delivered.encode_log_data() },
                transaction_hash: Some(l1_hash),
                ..Default::default()
            };
            let l1_responses = Asserter::new();
            l1_responses.push_success(&"0xa");
            let mut removed = delivered_log.clone();
            removed.removed = true;
            let unrelated = IZonePortal::WithdrawalProcessed { senderTag: B256::ZERO, ..delivered };
            let mut unrelated_log = delivered_log.clone();
            unrelated_log.inner.data = unrelated.encode_log_data();
            let mut logs = vec![removed, unrelated_log];
            if success.is_some() {
                logs.push(delivered_log);
            }
            l1_responses.push_success(&logs);
            let l1 = ProviderBuilder::new()
                .network::<TempoNetwork>()
                .connect_mocked_client(l1_responses);
            let result = tokio::time::timeout(
                Duration::from_millis(100),
                wait_for_withdrawal(&zone, &l1, portal, 10, 20, zone_hash),
            )
            .await;
            match success {
                Some(true) => assert_eq!(result.unwrap().unwrap(), l1_hash),
                Some(false) => assert_eq!(
                    result.unwrap().unwrap_err().to_string(),
                    format!(
                        "L1 delivery failed in {l1_hash}; a refund was queued for the zone fallback recipient"
                    )
                ),
                None => assert!(
                    result.is_err(),
                    "unrelated or removed events must not complete the wait"
                ),
            }
        }
    }
}
