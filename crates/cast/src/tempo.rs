use alloy_primitives::Address;
use alloy_provider::Provider;
use tempo_alloy::{TempoNetwork, provider::TempoProviderExt};

pub use foundry_wallets::tempo::sign_with_access_key;

/// Checks whether an access key is already provisioned on-chain.
///
/// Queries the AccountKeychain precompile's `getKey` function. A key is considered
/// provisioned if the returned `keyId` is non-zero (i.e. the key exists and has not
/// been revoked).
pub async fn is_key_provisioned<P: Provider<TempoNetwork>>(
    provider: &P,
    wallet_address: Address,
    key_address: Address,
) -> bool {
    match provider.get_keychain_key(wallet_address, key_address).await {
        Ok(info) => info.keyId != Address::ZERO,
        Err(_) => false,
    }
}
