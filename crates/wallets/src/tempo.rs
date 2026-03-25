use alloy_eips::Encodable2718;
use alloy_primitives::Address;
use alloy_signer::Signer;
use eyre::Result;
use foundry_common::tempo;

use tempo_alloy::rpc::TempoTransactionRequest;
use tempo_primitives::transaction::{
    KeychainSignature, PrimitiveSignature, SignedKeyAuthorization, TempoSignature,
};

use crate::{WalletSigner, utils};

/// Configuration for a Tempo access key (keychain mode).
///
/// When a Tempo wallet entry uses keychain mode (`wallet_address != key_address`), the signer
/// is an access key that signs on behalf of the root wallet. This struct carries the metadata
/// needed to construct the correct transaction.
#[derive(Debug, Clone)]
pub struct TempoAccessKeyConfig {
    /// The root wallet address (the `from` address for transactions).
    pub wallet_address: Address,
    /// The access key's address (derived from the private key that actually signs).
    pub key_address: Address,
    /// Decoded key authorization for on-chain provisioning.
    ///
    /// When present, callers should check whether the key is already provisioned on-chain
    /// (via the AccountKeychain precompile) before including this in a transaction.
    pub key_authorization: Option<SignedKeyAuthorization>,
}

/// Result of looking up an address in Tempo's key store.
pub enum TempoLookup {
    /// A direct (EOA) signer was found — `wallet_address == key_address`.
    Direct(WalletSigner),
    /// A keychain (access key) signer was found — `wallet_address != key_address`.
    Keychain(WalletSigner, Box<TempoAccessKeyConfig>),
    /// No matching entry was found.
    NotFound,
}

/// Looks up a signer for the given address in Tempo's `keys.toml`.
///
/// Returns [`TempoLookup::Direct`] if a direct-mode (EOA) key is found,
/// [`TempoLookup::Keychain`] if a keychain-mode access key is found,
/// or [`TempoLookup::NotFound`] if no entry matches.
pub fn lookup_signer(from: Address) -> Result<TempoLookup> {
    let Some(file) = tempo::read_tempo_keys_file() else {
        return Ok(TempoLookup::NotFound);
    };

    for entry in &file.keys {
        if entry.wallet_address != from {
            continue;
        }

        let Some(key) = &entry.key else {
            continue;
        };

        // Direct mode: wallet_address == key_address (or key_address is absent).
        let is_direct =
            entry.key_address.is_none() || entry.key_address == Some(entry.wallet_address);

        let signer = utils::create_private_key_signer(key)?;

        if is_direct {
            return Ok(TempoLookup::Direct(signer));
        }

        // Keychain mode: the key is an access key signing on behalf of wallet_address.
        let key_authorization = entry
            .key_authorization
            .as_deref()
            .map(tempo::decode_key_authorization::<SignedKeyAuthorization>)
            .transpose()?;

        let config = TempoAccessKeyConfig {
            wallet_address: entry.wallet_address,
            // SAFETY: `is_direct` was false, so `key_address` is `Some` and != wallet_address
            key_address: entry.key_address.unwrap(),
            key_authorization,
        };
        return Ok(TempoLookup::Keychain(signer, Box::new(config)));
    }

    Ok(TempoLookup::NotFound)
}

/// Signs a Tempo transaction request using an access key (keychain V2 mode).
///
/// Bypasses the standard `EthereumWallet` signing path and instead:
/// 1. Builds the `TempoTransaction` from the request
/// 2. Computes the V2 keychain signing hash
/// 3. Signs with the access key
/// 4. Wraps in a `KeychainSignature` and encodes to EIP-2718 wire format
pub async fn sign_with_access_key(
    tx_request: impl Into<TempoTransactionRequest>,
    signer: &impl Signer,
    wallet_address: Address,
) -> Result<Vec<u8>> {
    let tx_request: TempoTransactionRequest = tx_request.into();
    let tempo_tx = tx_request
        .build_aa()
        .map_err(|e| eyre::eyre!("failed to build Tempo AA transaction: {e}"))?;

    let sig_hash = tempo_tx.signature_hash();
    let signing_hash = KeychainSignature::signing_hash(sig_hash, wallet_address);
    let raw_sig = signer.sign_hash(&signing_hash).await?;

    let keychain_sig =
        KeychainSignature::new(wallet_address, PrimitiveSignature::Secp256k1(raw_sig));
    let aa_signed = tempo_tx.into_signed(TempoSignature::Keychain(keychain_sig));

    let mut buf = Vec::new();
    aa_signed.encode_2718(&mut buf);

    Ok(buf)
}
