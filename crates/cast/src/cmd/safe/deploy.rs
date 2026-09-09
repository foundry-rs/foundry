use super::{
    contracts::{
        COMPATIBILITY_FALLBACK_HANDLER_V1_4_1, ISafe, ISafeProxyFactory, PREDETERMINED_SALT_NONCE,
        SAFE_L2_V1_4_1, SAFE_PROXY_FACTORY_V1_4_1, SAFE_V1_4_1, SENTINEL_OWNER,
    },
    rpc_provider,
    transaction::SafeSendOpts,
};
use alloy_network::Ethereum;
use alloy_primitives::{Address, Bytes, U256, keccak256, map::AddressHashSet};
use alloy_provider::Provider;
use alloy_sol_types::{SolCall, SolEvent};
use clap::Args;
use eyre::{Context, Result, ensure};
use foundry_cli::json::print_scalar;
use foundry_common::sh_status;

/// CLI arguments for `cast safe create`.
#[derive(Args, Debug)]
pub struct CreateArgs {
    /// Addresses that own the Safe.
    #[arg(required = true, num_args = 1..)]
    owners: Vec<Address>,

    /// Number of owner signatures required. Defaults to all owners.
    #[arg(long)]
    threshold: Option<usize>,

    /// CREATE2 salt nonce. Defaults to Safe Protocol Kit's chain-specific nonce.
    #[arg(long)]
    salt_nonce: Option<U256>,

    /// Safe singleton address. Defaults to the canonical v1.4.1 deployment.
    #[arg(long, conflicts_with = "l1")]
    singleton: Option<Address>,

    /// Use the L1 Safe singleton instead of SafeL2.
    #[arg(long)]
    l1: bool,

    /// SafeProxyFactory address.
    #[arg(long, default_value_t = SAFE_PROXY_FACTORY_V1_4_1)]
    factory: Address,

    /// CompatibilityFallbackHandler address. Pass the zero address to disable it.
    #[arg(long, default_value_t = COMPATIBILITY_FALLBACK_HANDLER_V1_4_1)]
    fallback_handler: Address,

    /// Number of confirmations to wait for.
    #[arg(long, default_value = "1")]
    confirmations: u64,

    /// Timeout for deployment confirmation, in seconds.
    #[arg(long, env = "ETH_TIMEOUT")]
    timeout: Option<u64>,

    /// Polling interval for the deployment receipt, in seconds.
    #[arg(long, env = "ETH_POLL_INTERVAL")]
    poll_interval: Option<u64>,

    #[command(flatten)]
    send: SafeSendOpts,
}

impl CreateArgs {
    pub(super) async fn run(self) -> Result<()> {
        let Self {
            owners,
            threshold,
            salt_nonce,
            singleton,
            l1,
            factory,
            fallback_handler,
            confirmations,
            timeout,
            poll_interval,
            send,
        } = self;
        let threshold = validate_owners(&owners, threshold)?;
        let (provider, chain_id) = rpc_provider(&send.rpc).await?;
        let singleton =
            singleton.unwrap_or(if l1 || chain_id == 1 { SAFE_V1_4_1 } else { SAFE_L2_V1_4_1 });
        ensure_contract(&provider, singleton, "Safe singleton", "--singleton").await?;
        ensure_contract(&provider, factory, "SafeProxyFactory", "--factory").await?;
        if fallback_handler != Address::ZERO {
            ensure_contract(
                &provider,
                fallback_handler,
                "CompatibilityFallbackHandler",
                "--fallback-handler",
            )
            .await?;
        }

        let initializer = ISafe::setupCall {
            owners,
            threshold: U256::from(threshold),
            to: Address::ZERO,
            data: Bytes::new(),
            fallbackHandler: fallback_handler,
            paymentToken: Address::ZERO,
            payment: U256::ZERO,
            paymentReceiver: Address::ZERO,
        }
        .abi_encode()
        .into();

        sh_status!("Deploying Safe with singleton {singleton}")?;
        let calldata = ISafeProxyFactory::createProxyWithNonceCall {
            singleton,
            initializer,
            saltNonce: salt_nonce.unwrap_or_else(|| default_salt_nonce(chain_id)),
        }
        .abi_encode()
        .into();
        let result = send
            .send(factory, calldata, confirmations, timeout, poll_interval)
            .await
            .wrap_err("failed to submit Safe deployment")?;
        let deployed = result
            .logs
            .iter()
            .filter(|log| log.address() == factory)
            .find_map(|log| ISafeProxyFactory::ProxyCreation::decode_log(&log.inner).ok())
            .ok_or_else(|| eyre::eyre!("Safe deployment receipt did not emit ProxyCreation"))?;
        sh_status!("Transaction hash: {}", result.tx_hash)?;
        print_scalar(deployed.proxy.to_checksum(None))
    }
}

fn validate_owners(owners: &[Address], threshold: Option<usize>) -> Result<usize> {
    ensure!(!owners.is_empty(), "at least one Safe owner is required");
    let threshold = threshold.unwrap_or(owners.len());
    ensure!(threshold > 0, "Safe threshold must be greater than zero");
    ensure!(
        threshold <= owners.len(),
        "Safe threshold ({threshold}) exceeds owner count ({})",
        owners.len()
    );
    let mut unique = AddressHashSet::default();
    for owner in owners {
        ensure!(*owner != Address::ZERO, "Safe owner cannot be the zero address");
        ensure!(*owner != SENTINEL_OWNER, "Safe owner cannot be the sentinel address");
        ensure!(unique.insert(*owner), "duplicate Safe owner: {owner}");
    }
    Ok(threshold)
}

fn default_salt_nonce(chain_id: u64) -> U256 {
    U256::from_be_slice(keccak256(format!("{PREDETERMINED_SALT_NONCE}{chain_id}")).as_slice())
}

pub(super) async fn ensure_contract<P: Provider<Ethereum>>(
    provider: &P,
    address: Address,
    name: &str,
    flag: &str,
) -> Result<()> {
    ensure!(
        !provider.get_code_at(address).await?.is_empty(),
        "{name} is not deployed at {address}; provide the network deployment with {flag}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn validates_safe_owner_configuration() {
        let owners = [Address::repeat_byte(1), Address::repeat_byte(2)];
        assert_eq!(validate_owners(&owners, None).unwrap(), 2);
        assert_eq!(validate_owners(&owners, Some(1)).unwrap(), 1);
        assert!(validate_owners(&owners, Some(3)).is_err());
        assert!(validate_owners(&[owners[0], owners[0]], None).is_err());
        assert!(validate_owners(&[Address::ZERO], None).is_err());
    }

    #[test]
    fn matches_protocol_kit_default_salt_nonce() {
        assert_eq!(
            default_salt_nonce(1),
            U256::from_str("0x69b348339eea4ed93f9d11931c3b894c8f9d8c7663a053024b11cb7eb4e5a1f6")
                .unwrap()
        );
    }
}
