use alloy_primitives::Address;
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
}
