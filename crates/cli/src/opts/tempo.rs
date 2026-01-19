use alloy_primitives::{Address, ruint::aliases::U256};
use clap::Parser;

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
    #[arg(long = "tempo.fee-token")]
    pub fee_token: Option<Address>,

    /// Nonce sequence key for Tempo transactions.
    ///
    /// When set, builds a Tempo (type 0x76) transaction with the specified nonce sequence key.
    ///
    /// If this is not set, the protocol sequence key (0) will be used.
    ///
    /// For more information see <https://docs.tempo.xyz/protocol/transactions/spec-tempo-transaction#parallelizable-nonces>.
    #[arg(long = "tempo.seq")]
    pub sequence_key: Option<U256>,
}
