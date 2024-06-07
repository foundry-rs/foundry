use clap::{builder::RangedU64ValueParser, Parser};
use foundry_common::retry::Retry;
use std::time::Duration;

/// Retry config used when waiting for verification
pub const RETRY_CHECK_ON_VERIFY: RetryArgs = RetryArgs { retries: 8, delay: 15 };

/// Retry config used when waiting for a created contract
pub const RETRY_VERIFY_ON_CREATE: RetryArgs = RetryArgs { retries: 15, delay: 5 };

/// Retry arguments for contract verification.
#[derive(Clone, Copy, Debug, Parser)]
#[command(about = "Allows to use retry arguments for contract verification")] // override doc
pub struct RetryArgs {
    /// Number of attempts for retrying verification.
    #[arg(
        long,
        value_parser = RangedU64ValueParser::<u32>::new().range(1..),
        default_value = "5",
    )]
    pub retries: u32,

    /// Optional delay to apply inbetween verification attempts, in seconds.
    #[arg(
        long,
        value_parser = RangedU64ValueParser::<u32>::new().range(0..=30),
        default_value = "5",
    )]
    pub delay: u32,
}

impl Default for RetryArgs {
    fn default() -> Self {
        RETRY_VERIFY_ON_CREATE
    }
}

impl From<RetryArgs> for Retry {
    fn from(r: RetryArgs) -> Self {
        Self::new(r.retries, Some(Duration::from_secs(r.delay as u64)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli() {
        let args = RetryArgs::parse_from(["foundry-cli", "--retries", "10"]);
        assert_eq!(args.retries, 10);
        assert_eq!(args.delay, 5);

        let args = RetryArgs::parse_from(["foundry-cli", "--delay", "10"]);
        assert_eq!(args.retries, 5);
        assert_eq!(args.delay, 10);

        let args = RetryArgs::parse_from(["foundry-cli", "--retries", "10", "--delay", "10"]);
        assert_eq!(args.retries, 10);
        assert_eq!(args.delay, 10);
    }
}
