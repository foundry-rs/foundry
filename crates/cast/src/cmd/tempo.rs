use clap::Parser;
use eyre::Result;
use foundry_common::tempo::{EnsureAccessKeyConfig, ensure_access_key};

/// Tempo wallet integration commands.
#[derive(Debug, Parser)]
pub enum TempoSubcommand {
    /// Authorize a new access key against your Tempo wallet via wallet.tempo.
    ///
    /// Persists the key to `$TEMPO_HOME/wallet/keys.toml` (default
    /// `~/.tempo/wallet/keys.toml`). Also runs automatically on a 402 from a
    /// Tempo RPC when no local key is configured.
    ///
    /// Env: `TEMPO_HOME`, `TEMPO_CLI_AUTH_URL` (override auth service).
    Login {
        /// Chain ID to authorize the key for. Defaults to Tempo mainnet (4217).
        #[arg(long, default_value_t = 4217)]
        chain_id: u64,

        /// Print the authorization URL to stderr instead of opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
}

impl TempoSubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Login { chain_id, no_browser } => {
                let mut cfg = EnsureAccessKeyConfig::from_env(chain_id);
                if no_browser {
                    cfg.no_browser = true;
                }
                let outcome = ensure_access_key(cfg).await?;
                let _ = foundry_common::sh_println!(
                    "Authorized key {} for wallet {} on chain {}",
                    outcome.key_address,
                    outcome.wallet_address,
                    outcome.chain_id,
                );
                Ok(())
            }
        }
    }
}
