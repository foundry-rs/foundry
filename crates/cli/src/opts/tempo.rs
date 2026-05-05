use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{Address, ruint::aliases::U256};
use alloy_signer::Signature;
use clap::Parser;
use foundry_common::FoundryTransactionBuilder;
use std::{
    num::NonZeroU64,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::utils::parse_fee_token_address;

/// CLI options common to Tempo transactions across commands.
#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Tempo")]
pub struct TempoCommonOpts {
    /// Fee token address for Tempo transactions.
    ///
    /// When set, builds a Tempo (type 0x76) transaction that pays gas fees
    /// in the specified token.
    ///
    /// If this is not set, the fee token is chosen according to network rules. See the Tempo docs
    /// for more information.
    #[arg(long = "tempo.fee-token", value_parser = parse_fee_token_address)]
    pub fee_token: Option<Address>,

    /// Opt into TIP-1009 expiring-nonce mode with a validity window.
    ///
    /// Convenience flag that combines `--tempo.expiring-nonce` with a relative
    /// `--tempo.valid-before`. Sets nonce_key = U256::MAX, nonce = 0, and valid_before = now +
    /// seconds.
    ///
    /// Maximum value is 30 seconds. The transaction must be mined before the deadline or it
    /// becomes permanently invalid, giving safe retry semantics: retries produce a fresh tx hash
    /// and the old tx can never land late.
    #[arg(long = "tempo.expires", value_name = "SECONDS", value_parser = parse_expires_seconds)]
    pub expires: Option<u64>,
}

impl TempoCommonOpts {
    /// Returns `true` if any Tempo-specific option is set.
    pub const fn is_tempo(&self) -> bool {
        self.fee_token.is_some() || self.expires.is_some()
    }

    /// Returns the absolute `valid_before` unix timestamp derived from `--tempo.expires`, if set.
    pub fn expires_at(&self) -> Option<u64> {
        let secs = self.expires?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards");
        Some(now.as_secs() + secs)
    }
}

/// CLI options for Tempo transactions.
#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Tempo")]
pub struct TempoOpts {
    #[command(flatten)]
    pub common: TempoCommonOpts,

    /// Nonce key for Tempo parallelizable nonces.
    ///
    /// When set, builds a Tempo (type 0x76) transaction with the specified nonce key,
    /// allowing multiple transactions with the same nonce but different keys
    /// to be executed in parallel. If not set, the protocol nonce key (0) will be used.
    ///
    /// For more information see <https://docs.tempo.xyz/protocol/transactions/spec-tempo-transaction#parallelizable-nonces>.
    #[arg(long = "tempo.nonce-key", value_name = "NONCE_KEY")]
    pub nonce_key: Option<U256>,

    /// Sponsor (fee payer) signature for Tempo sponsored transactions.
    ///
    /// The sponsor signs the `fee_payer_signature_hash` to commit to paying gas fees
    /// on behalf of the sender. Provide as a hex-encoded signature.
    #[arg(long = "tempo.sponsor-signature", value_parser = parse_signature)]
    pub sponsor_signature: Option<Signature>,

    /// Print the sponsor signature hash and exit.
    ///
    /// Computes the `fee_payer_signature_hash` for the transaction so that a sponsor
    /// knows what hash to sign. The transaction is not sent.
    #[arg(long = "tempo.print-sponsor-hash")]
    pub print_sponsor_hash: bool,

    /// Access key ID for Tempo Keychain signature transactions.
    ///
    /// Used during gas estimation to override the key_id that would normally be
    /// recovered from the signature.
    #[arg(long = "tempo.key-id")]
    pub key_id: Option<Address>,

    /// Enable expiring nonce mode for Tempo transactions.
    ///
    /// Sets nonce to 0 and nonce_key to U256::MAX, enabling time-bounded transaction
    /// validity via `--tempo.valid-before` and `--tempo.valid-after`.
    #[arg(long = "tempo.expiring-nonce", requires = "valid_before", conflicts_with = "expires")]
    pub expiring_nonce: bool,

    /// Upper bound timestamp for Tempo expiring nonce transactions.
    ///
    /// The transaction is only valid before this unix timestamp.
    /// Requires `--tempo.expiring-nonce`.
    #[arg(long = "tempo.valid-before", conflicts_with = "expires")]
    pub valid_before: Option<u64>,

    /// Lower bound timestamp for Tempo expiring nonce transactions.
    ///
    /// The transaction is only valid after this unix timestamp.
    /// Requires `--tempo.expiring-nonce`.
    #[arg(long = "tempo.valid-after")]
    pub valid_after: Option<u64>,
}

impl TempoOpts {
    /// Returns `true` if any Tempo-specific option is set.
    pub const fn is_tempo(&self) -> bool {
        self.common.is_tempo()
            || self.nonce_key.is_some()
            || self.sponsor_signature.is_some()
            || self.print_sponsor_hash
            || self.key_id.is_some()
            || self.expiring_nonce
            || self.valid_before.is_some()
            || self.valid_after.is_some()
    }

    /// Returns the absolute `valid_before` unix timestamp derived from `--tempo.expires`, if set.
    pub fn expires_at(&self) -> Option<u64> {
        self.common.expires_at()
    }

    /// Applies Tempo-specific options to a transaction request.
    ///
    /// All setters are no-ops for non-Tempo networks, so this is safe to call unconditionally.
    pub fn apply<N: Network>(&self, tx: &mut N::TransactionRequest, nonce: Option<u64>)
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        // Handle expiring nonce mode: sets nonce=0 and nonce_key=U256::MAX.
        // --tempo.expires is a convenience alias that also sets valid_before = now + duration.
        if self.expiring_nonce || self.common.expires.is_some() {
            tx.set_nonce(0);
            tx.set_nonce_key(U256::MAX);
        } else {
            if let Some(nonce) = nonce {
                tx.set_nonce(nonce);
            }
            if let Some(nonce_key) = self.nonce_key {
                tx.set_nonce_key(nonce_key);
            }
        }

        if let Some(fee_token) = self.common.fee_token {
            tx.set_fee_token(fee_token);
        }

        // --tempo.expires sets valid_before relative to now; --tempo.valid-before takes a raw
        // unix timestamp. The two flags are mutually exclusive (enforced by clap).
        let effective_valid_before = self.expires_at().or(self.valid_before);
        if let Some(valid_before) = effective_valid_before
            && let Some(v) = NonZeroU64::new(valid_before)
        {
            tx.set_valid_before(v);
        }
        if let Some(valid_after) = self.valid_after
            && let Some(v) = NonZeroU64::new(valid_after)
        {
            tx.set_valid_after(v);
        }

        if let Some(key_id) = self.key_id {
            tx.set_key_id(key_id);
        }

        // Force AA tx type if sponsoring or printing sponsor hash.
        // Note: the fee_payer_signature is NOT set here. It must be applied AFTER
        // gas estimation so that `--tempo.print-sponsor-hash` and
        // `--tempo.sponsor-signature` produce identical gas estimates. Callers
        // should call `set_fee_payer_signature` on the built tx request.
        if (self.sponsor_signature.is_some() || self.print_sponsor_hash) && tx.nonce_key().is_none()
        {
            tx.set_nonce_key(U256::ZERO);
        }
    }
}

fn parse_signature(s: &str) -> Result<Signature, String> {
    Signature::from_str(s).map_err(|e| format!("invalid signature: {e}"))
}

/// Parses a seconds value for `--tempo.expires`, capped at the protocol maximum of 30 seconds.
fn parse_expires_seconds(s: &str) -> Result<u64, String> {
    let secs: u64 = s
        .parse()
        .map_err(|_| format!("invalid value '{s}': expected an integer number of seconds"))?;
    if secs > 30 {
        return Err(format!("expires must be at most 30 seconds (got {secs})"));
    }
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn parse_expires_flag() {
        let opts = TempoOpts::try_parse_from(["", "--tempo.expires", "30"]).unwrap();
        assert_eq!(opts.common.expires, Some(30));

        let opts = TempoOpts::try_parse_from(["", "--tempo.expires", "10"]).unwrap();
        assert_eq!(opts.common.expires, Some(10));

        // exceeds 30s maximum
        assert!(TempoOpts::try_parse_from(["", "--tempo.expires", "31"]).is_err());

        // conflicts with --tempo.expiring-nonce
        assert!(
            TempoOpts::try_parse_from([
                "",
                "--tempo.expires",
                "30",
                "--tempo.expiring-nonce",
                "--tempo.valid-before",
                "999"
            ])
            .is_err()
        );
    }

    #[test]
    fn parse_fee_token_id() {
        let opts = TempoOpts::try_parse_from([
            "",
            "--tempo.fee-token",
            "0x20C0000000000000000000000000000000000002",
        ])
        .unwrap();
        assert_eq!(
            opts.common.fee_token,
            Some(address!("0x20C0000000000000000000000000000000000002")),
        );

        // AlphaUSD token ID is 1u64
        let opts_with_id = TempoOpts::try_parse_from(["", "--tempo.fee-token", "1"]).unwrap();
        assert_eq!(
            opts_with_id.common.fee_token,
            Some(address!("0x20C0000000000000000000000000000000000001")),
        );
    }
}
