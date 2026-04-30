//! Tempo network utilities.

mod keystore;
pub use keystore::*;

#[cfg(test)]
mod tests;

/// Conservative gas buffer for browser wallet transactions on Tempo chains.
///
/// Browser wallets may sign with P256 or WebAuthn instead of secp256k1, which costs more gas
/// for signature verification. Since we can't determine the signature type before signing,
/// we add the worst-case (WebAuthn) overhead:
///   - P256: +5,000 gas (P256 precompile cost minus ecrecover savings)
///   - WebAuthn: ~6,500 gas (P256 cost + calldata for webauthn_data)
///
/// See <https://github.com/tempoxyz/tempo/blob/6ebf1a8/crates/revm/src/handler.rs#L108-L124>
pub const TEMPO_BROWSER_GAS_BUFFER: u64 = 7_000;
