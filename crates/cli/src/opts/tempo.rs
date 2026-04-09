use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{Address, ruint::aliases::U256};
use alloy_signer::Signature;
use clap::Parser;
use foundry_common::FoundryTransactionBuilder;
use std::str::FromStr;

use crate::utils::parse_fee_token_address;

/// CLI options for Tempo transactions.
#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Tempo")]
pub struct TempoOpts {
    /// Fee token address for Tempo transactions.
    ///
    /// When set, builds a Tempo (type 0x76) transaction that pays gas fees
    /// in the specified token.
    ///
    /// If this is not set, the fee token is chosen according to network rules. See the Tempo docs
    /// for more information.
    #[arg(long = "tempo.fee-token", value_parser = parse_fee_token_address)]
    pub fee_token: Option<Address>,

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
    #[arg(long = "tempo.expiring-nonce", requires = "valid_before")]
    pub expiring_nonce: bool,

    /// Upper bound timestamp for Tempo expiring nonce transactions.
    ///
    /// The transaction is only valid before this unix timestamp.
    /// Requires `--tempo.expiring-nonce`.
    #[arg(long = "tempo.valid-before")]
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
    pub fn is_tempo(&self) -> bool {
        self.fee_token.is_some()
            || self.nonce_key.is_some()
            || self.sponsor_signature.is_some()
            || self.print_sponsor_hash
            || self.key_id.is_some()
            || self.expiring_nonce
            || self.valid_before.is_some()
            || self.valid_after.is_some()
    }

    /// Applies Tempo-specific options to a transaction request.
    ///
    /// All setters are no-ops for non-Tempo networks, so this is safe to call unconditionally.
    pub fn apply<N: Network>(&self, tx: &mut N::TransactionRequest, nonce: Option<u64>)
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        // Handle expiring nonce mode: sets nonce=0 and nonce_key=U256::MAX
        if self.expiring_nonce {
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

        if let Some(fee_token) = self.fee_token {
            tx.set_fee_token(fee_token);
        }

        if let Some(valid_before) = self.valid_before {
            tx.set_valid_before(valid_before);
        }
        if let Some(valid_after) = self.valid_after {
            tx.set_valid_after(valid_after);
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

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_evm::core::tempo::{ALPHA_USD_ADDRESS, BETA_USD_ADDRESS};

    #[test]
    fn parse_fee_token_id() {
        let opts = TempoOpts::try_parse_from([
            "",
            "--tempo.fee-token",
            "0x20C0000000000000000000000000000000000002",
        ])
        .unwrap();
        assert_eq!(opts.fee_token, Some(BETA_USD_ADDRESS),);

        // AlphaUSD token ID is 1u64
        let opts_with_id = TempoOpts::try_parse_from(["", "--tempo.fee-token", "1"]).unwrap();
        assert_eq!(opts_with_id.fee_token, Some(ALPHA_USD_ADDRESS),);
    }
}
