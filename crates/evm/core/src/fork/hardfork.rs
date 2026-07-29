//! Hardfork resolution for remote forks.

use alloy_chains::Chain;
use alloy_network::Network;
use alloy_primitives::{Address, Bytes, U256, address, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork, TempoHardfork};

#[cfg(feature = "optimism")]
use foundry_evm_hardforks::OpHardfork;

const ARB_SYS_ADDRESS: Address = address!("0000000000000000000000000000000000000064");
const ARB_OS_VERSION_SELECTOR: Bytes = bytes!("051038f2");
const ARB_OS_VERSION_OFFSET: u64 = 55;

/// Resolves the hardfork active at a remote fork block.
///
/// Explicit configuration takes precedence over known Ethereum and OP Stack schedules. Arbitrum
/// Nitro chains are resolved by querying `ArbSys.arbOSVersion()` at the pinned block. Any
/// unsupported chain or failed best-effort lookup retains `fallback`.
pub async fn resolve_fork_hardfork<N, P>(
    provider: &P,
    configured: Option<FoundryHardfork>,
    fallback: FoundryHardfork,
    chain_id: u64,
    timestamp: u64,
    block_number: u64,
) -> FoundryHardfork
where
    N: Network,
    P: Provider<N>,
{
    if let Some(hardfork) = configured {
        return hardfork;
    }

    let chain = Chain::from_id(chain_id);
    if !chain.is_arbitrum()
        && let Some(hardfork) = EthereumHardfork::from_chain_and_timestamp(chain, timestamp)
    {
        return hardfork.into();
    }

    #[cfg(feature = "optimism")]
    if chain.is_optimism()
        && let Some(hardfork) = OpHardfork::from_chain_and_timestamp(chain, timestamp)
    {
        return hardfork.into();
    }

    if chain.is_tempo()
        && let Some(hardfork) = TempoHardfork::from_chain_and_timestamp(chain_id, timestamp)
    {
        return hardfork.into();
    }

    if chain.is_arbitrum()
        && let Ok(result) = provider
            .raw_request::<_, Bytes>(
                "eth_call".into(),
                (
                    serde_json::json!({
                        "to": ARB_SYS_ADDRESS,
                        "data": ARB_OS_VERSION_SELECTOR,
                    }),
                    BlockId::number(block_number),
                ),
            )
            .await
        && result.len() == 32
        && let Some(hardfork) = arbitrum_hardfork(U256::from_be_slice(&result))
    {
        return hardfork.into();
    }

    fallback
}

fn arbitrum_hardfork(arb_os_version: U256) -> Option<EthereumHardfork> {
    // ArbSys exposes internal ArbOS versions with a 55 offset.
    let arb_os_version = arb_os_version.checked_sub(U256::from(ARB_OS_VERSION_OFFSET))?;

    if arb_os_version >= U256::from(50) {
        Some(EthereumHardfork::Osaka)
    } else if arb_os_version >= U256::from(40) {
        Some(EthereumHardfork::Prague)
    } else if arb_os_version >= U256::from(20) {
        Some(EthereumHardfork::Cancun)
    } else if arb_os_version >= U256::from(11) {
        Some(EthereumHardfork::Shanghai)
    } else if arb_os_version >= U256::from(1) {
        Some(EthereumHardfork::Paris)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_hardforks::hoodi::HOODI_PRAGUE_TIMESTAMP;
    use alloy_network::AnyNetwork;
    use alloy_provider::{ProviderBuilder, mock::Asserter};

    fn provider(asserter: Asserter) -> impl Provider<AnyNetwork> {
        ProviderBuilder::new_with_network::<AnyNetwork>().connect_mocked_client(asserter)
    }

    #[tokio::test]
    async fn explicit_hardfork_takes_precedence() {
        let hardfork = EthereumHardfork::Berlin.into();
        let resolved = resolve_fork_hardfork(
            &provider(Asserter::new()),
            Some(hardfork),
            EthereumHardfork::Osaka.into(),
            42161,
            u64::MAX,
            123,
        )
        .await;

        assert_eq!(resolved, hardfork);
    }

    #[tokio::test]
    async fn resolves_known_chain_schedules_before_rpc() {
        let fallback = EthereumHardfork::Osaka.into();
        let provider = provider(Asserter::new());

        let ethereum = resolve_fork_hardfork(&provider, None, fallback, 1, 0, 123).await;
        assert_eq!(ethereum, EthereumHardfork::Frontier.into());

        #[cfg(feature = "optimism")]
        {
            let optimism =
                resolve_fork_hardfork(&provider, None, fallback, 10, u64::MAX, 123).await;
            assert_eq!(optimism, OpHardfork::Jovian.into());
        }
    }

    #[tokio::test]
    async fn resolves_hoodi_hardfork_boundary() {
        let fallback = EthereumHardfork::Osaka.into();
        let provider = provider(Asserter::new());

        let before = resolve_fork_hardfork(
            &provider,
            None,
            fallback,
            560048,
            HOODI_PRAGUE_TIMESTAMP - 1,
            123,
        )
        .await;
        assert_eq!(before, EthereumHardfork::Cancun.into());

        let at =
            resolve_fork_hardfork(&provider, None, fallback, 560048, HOODI_PRAGUE_TIMESTAMP, 123)
                .await;
        assert_eq!(at, EthereumHardfork::Prague.into());
    }

    #[test]
    fn maps_arb_sys_os_versions_in_descending_order() {
        let cases = [
            (54, None),
            (55, None),
            (56, Some(EthereumHardfork::Paris)),
            (65, Some(EthereumHardfork::Paris)),
            (66, Some(EthereumHardfork::Shanghai)),
            (74, Some(EthereumHardfork::Shanghai)),
            (75, Some(EthereumHardfork::Cancun)),
            (94, Some(EthereumHardfork::Cancun)),
            (95, Some(EthereumHardfork::Prague)),
            (104, Some(EthereumHardfork::Prague)),
            (105, Some(EthereumHardfork::Osaka)),
            (u64::MAX, Some(EthereumHardfork::Osaka)),
        ];

        for (version, expected) in cases {
            assert_eq!(arbitrum_hardfork(U256::from(version)), expected);
        }
    }

    #[tokio::test]
    async fn falls_back_for_unsupported_arb_os_version() {
        let asserter = Asserter::new();
        asserter.push_success(&Bytes::from([0u8; 32]));
        let fallback = EthereumHardfork::London.into();

        let resolved =
            resolve_fork_hardfork(&provider(asserter), None, fallback, 42170, u64::MAX, 123).await;

        assert_eq!(resolved, fallback);
    }

    #[tokio::test]
    async fn falls_back_when_arb_os_lookup_fails() {
        let asserter = Asserter::new();
        asserter.push_failure_msg("historical state unavailable");
        let fallback = EthereumHardfork::London.into();

        let resolved =
            resolve_fork_hardfork(&provider(asserter), None, fallback, 42170, u64::MAX, 123).await;

        assert_eq!(resolved, fallback);
    }

    #[tokio::test]
    async fn resolves_arbitrum_hardfork_from_arb_os_version() {
        let asserter = Asserter::new();
        asserter.push_success(&Bytes::copy_from_slice(&U256::from(75).to_be_bytes::<32>()));

        let resolved = resolve_fork_hardfork(
            &provider(asserter),
            None,
            EthereumHardfork::Osaka.into(),
            42170,
            u64::MAX,
            123,
        )
        .await;

        assert_eq!(resolved, EthereumHardfork::Cancun.into());
    }

    #[tokio::test]
    async fn unknown_chain_retains_fallback() {
        let fallback = EthereumHardfork::London.into();
        let resolved = resolve_fork_hardfork(
            &provider(Asserter::new()),
            None,
            fallback,
            9_999_999,
            u64::MAX,
            123,
        )
        .await;

        assert_eq!(resolved, fallback);
    }
}
