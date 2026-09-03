use super::{
    contracts::{
        ISafe, ISafeProxyFactory, PREDETERMINED_SALT_NONCE, SAFE_L2_V1_4_1, SAFE_V1_4_1,
        SENTINEL_OWNER,
    },
    transaction::send_safe_call,
};
use alloy_network::Ethereum;
use alloy_primitives::{Address, Bytes, U256, keccak256, map::AddressHashSet};
use alloy_provider::Provider;
use alloy_sol_types::{SolCall, SolEvent};
use eyre::{Context, Result, ensure};
use foundry_cli::{
    json::print_scalar,
    opts::{RpcOpts, TransactionOpts},
    utils::LoadConfig,
};
use foundry_common::{provider::ProviderBuilder, sh_status};
use foundry_wallets::WalletOpts;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    owners: Vec<Address>,
    threshold: Option<usize>,
    salt_nonce: Option<U256>,
    singleton: Option<Address>,
    l1: bool,
    factory: Address,
    fallback_handler: Address,
    confirmations: u64,
    timeout: Option<u64>,
    poll_interval: Option<u64>,
    rpc: RpcOpts,
    wallet: WalletOpts,
    tx: TransactionOpts,
) -> Result<()> {
    let threshold = validate_owners(&owners, threshold)?;
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain_id = provider.get_chain_id().await?;
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

    let salt_nonce = salt_nonce.unwrap_or_else(|| default_salt_nonce(chain_id));
    let initializer: Bytes = ISafe::setupCall {
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
    let calldata: Bytes = ISafeProxyFactory::createProxyWithNonceCall {
        singleton,
        initializer,
        saltNonce: salt_nonce,
    }
    .abi_encode()
    .into();
    let result =
        send_safe_call(factory, calldata, confirmations, timeout, poll_interval, rpc, wallet, tx)
            .await
            .wrap_err("failed to submit Safe deployment")?;
    let deployed = result
        .logs
        .iter()
        .filter(|log| log.address() == factory)
        .find_map(|log| ISafeProxyFactory::ProxyCreation::decode_log(&log.inner).ok())
        .ok_or_else(|| eyre::eyre!("Safe deployment receipt did not emit ProxyCreation"))?;
    sh_status!("Transaction hash: {}", result.tx_hash)?;
    print_scalar(deployed.proxy.to_checksum(None))?;
    Ok(())
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

pub(super) async fn ensure_contract<P>(
    provider: &P,
    address: Address,
    name: &str,
    flag: &str,
) -> Result<()>
where
    P: Provider<Ethereum>,
{
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
