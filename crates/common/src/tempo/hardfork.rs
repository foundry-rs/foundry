//! Tempo hardfork activation queries.

use crate::{FoundryTransactionBuilder, provider::is_rpc_method_not_found};
use alloy_network::{Network, TransactionBuilder};
use alloy_provider::Provider;
use alloy_transport::{TransportError, TransportErrorKind};
use eyre::Result;
use serde::Deserialize;
use tempo_alloy::{chainspec::hardfork::TempoHardfork, rpc::ForkSchedule};
use tempo_primitives::transaction::TEMPO_EXPIRING_NONCE_KEY;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnvilNodeInfo {
    hard_fork: Option<String>,
    network: Option<String>,
}

/// Returns whether `hardfork` is active at the head of the chain behind `provider`.
///
/// Queries `tempo_forkSchedule` and falls back to `anvil_nodeInfo` for Anvil nodes, which do not
/// serve the fork schedule. Fails when neither source reports a hardfork known to this build.
pub async fn is_tempo_hardfork_active<N: Network, P: Provider<N>>(
    provider: &P,
    hardfork: TempoHardfork,
) -> Result<bool> {
    match fork_schedule_hardfork_active(provider, hardfork).await {
        Ok(active) => Ok(active),
        Err(err) if is_rpc_method_not_found(&err) => {
            match anvil_tempo_hardfork_active(provider, hardfork).await {
                Ok(Some(active)) => Ok(active),
                _ => Err(err.into()),
            }
        }
        Err(err) => Err(err.into()),
    }
}

/// Fails when `tx` carries a non-zero expiring nonce and the chain is known to predate T12.
///
/// TIP-1106 turns the nonce of an expiring nonce transaction into an opaque discriminator from
/// T12 on. Earlier hardforks reject any value other than zero. When the active hardfork cannot be
/// determined the check is skipped and validation is left to the node.
pub async fn ensure_expiring_nonce_discriminator_active<N: Network, P: Provider<N>>(
    provider: &P,
    tx: &N::TransactionRequest,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    if tx.nonce_key() == Some(TEMPO_EXPIRING_NONCE_KEY)
        && let Some(nonce) = tx.nonce().filter(|nonce| *nonce != 0)
        && matches!(is_tempo_hardfork_active(provider, TempoHardfork::T12).await, Ok(false))
    {
        eyre::bail!(
            "expiring nonce transactions must use nonce 0 before the Tempo T12 hardfork, got nonce {nonce}; non-zero expiring nonce discriminators (TIP-1106) are not active on this chain"
        );
    }
    Ok(())
}

async fn fork_schedule_hardfork_active<N: Network, P: Provider<N>>(
    provider: &P,
    hardfork: TempoHardfork,
) -> Result<bool, TransportError> {
    let schedule = provider.raw_request::<_, ForkSchedule>("tempo_forkSchedule".into(), ()).await?;
    let active = schedule.active.parse::<TempoHardfork>().map_err(TransportErrorKind::custom)?;
    Ok(active >= hardfork)
}

async fn anvil_tempo_hardfork_active<N: Network, P: Provider<N>>(
    provider: &P,
    hardfork: TempoHardfork,
) -> Result<Option<bool>, TransportError> {
    let info = provider.raw_request::<_, AnvilNodeInfo>("anvil_nodeInfo".into(), ()).await?;
    Ok(active_from_anvil_node_info(&info, hardfork))
}

fn active_from_anvil_node_info(info: &AnvilNodeInfo, hardfork: TempoHardfork) -> Option<bool> {
    (info.network.as_deref() == Some("tempo")).then(|| {
        info.hard_fork
            .as_deref()
            .and_then(|active_hardfork| active_hardfork.parse::<TempoHardfork>().ok())
            .is_some_and(|active_hardfork| active_hardfork >= hardfork)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;
    use alloy_provider::{ProviderBuilder, mock::Asserter};
    use tempo_alloy::{TempoNetwork, rpc::TempoTransactionRequest};

    fn mocked_provider(asserter: Asserter) -> impl Provider<TempoNetwork> {
        ProviderBuilder::new().network::<TempoNetwork>().connect_mocked_client(asserter)
    }

    #[tokio::test]
    async fn tempo_fork_schedule_detects_t3_activation() {
        for (active, expected) in [("T2", false), ("T3", true), ("T13", true), ("T14", true)] {
            let asserter = Asserter::new();
            asserter.push_success(&serde_json::json!({ "active": active, "schedule": [] }));
            let provider = mocked_provider(asserter);
            assert_eq!(
                is_tempo_hardfork_active(&provider, TempoHardfork::T3).await.unwrap(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn tempo_fork_schedule_rejects_unknown_hardfork() {
        let asserter = Asserter::new();
        asserter.push_success(&serde_json::json!({ "active": "FutureFork", "schedule": [] }));
        let provider = mocked_provider(asserter);
        assert!(is_tempo_hardfork_active(&provider, TempoHardfork::T3).await.is_err());
    }

    #[test]
    fn active_from_anvil_node_info_requires_tempo_network() {
        let info = |network: &str, hard_fork: &str| AnvilNodeInfo {
            network: Some(network.to_string()),
            hard_fork: Some(hard_fork.to_string()),
        };
        let tempo_t3 = info("tempo", "T3");
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T2), Some(true));
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T3), Some(true));
        assert_eq!(active_from_anvil_node_info(&tempo_t3, TempoHardfork::T4), Some(false));
        assert_eq!(
            active_from_anvil_node_info(&info("tempo", "T11"), TempoHardfork::T11),
            Some(true)
        );
        assert_eq!(active_from_anvil_node_info(&info("ethereum", "T3"), TempoHardfork::T3), None);
    }

    #[tokio::test]
    async fn expiring_nonce_discriminator_requires_t12_when_hardfork_is_known() {
        let request = |nonce_key, nonce| {
            let mut tx = TempoTransactionRequest::default();
            tx.set_nonce_key(nonce_key);
            tx.set_nonce(nonce);
            tx
        };
        let discriminator = request(TEMPO_EXPIRING_NONCE_KEY, 7);

        // `None` leaves the mock without a response, so any hardfork query fails.
        for (tx, active, accepted) in [
            (&discriminator, Some("T11"), false),
            (&discriminator, Some("T12"), true),
            (&discriminator, Some("FutureFork"), true),
            (&discriminator, None, true),
            (&request(TEMPO_EXPIRING_NONCE_KEY, 0), None, true),
            (&request(U256::from(1), 7), None, true),
        ] {
            let asserter = Asserter::new();
            if let Some(active) = active {
                asserter.push_success(&serde_json::json!({ "active": active, "schedule": [] }));
            }
            let provider = mocked_provider(asserter);
            let result = ensure_expiring_nonce_discriminator_active(&provider, tx).await;
            assert_eq!(
                result.is_ok(),
                accepted,
                "nonce {:?} at {active:?}: {result:?}",
                tx.nonce()
            );
        }
    }
}
