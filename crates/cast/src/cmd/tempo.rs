use alloy_primitives::Address;
use alloy_signer_local::PrivateKeySigner;
use clap::Parser;
use eyre::Result;
use foundry_common::tempo::{EnsureAccessKeyConfig, decode_key_authorization, ensure_access_key};
use tempo_alloy::accounts::TempoAccountsStore;
use tempo_primitives::transaction::SignedKeyAuthorization;

/// Tempo wallet integration commands.
#[derive(Debug, Parser)]
#[allow(
    clippy::large_enum_variant,
    reason = "parsed once; retaining PrivateKeySigner keeps access-key input typed and redacted"
)]
pub enum TempoSubcommand {
    /// Authorize a new access key against your Tempo wallet via wallet.tempo.
    ///
    /// Persists the key to `$TEMPO_HOME/wallet/store.json` (default
    /// `~/.tempo/wallet/store.json`). Also runs automatically on a 402 from a
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

    /// Import a signed secp256k1 access key into the Tempo Accounts store.
    ///
    /// The signed authorization may be pending or already provisioned. The
    /// access-key private key is persisted to `store.json`; the authorizing
    /// root key is never stored.
    ImportAccessKey {
        /// Root Tempo account controlled by the access key.
        #[arg(long)]
        account: Address,

        /// Access-key private key to persist.
        #[arg(long, env = "TEMPO_ACCESS_KEY", hide_env_values = true)]
        access_key: PrivateKeySigner,

        /// Signed key authorization encoded as RLP hex.
        #[arg(long)]
        authorization: String,
    },
}

impl TempoSubcommand {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::Login { chain_id, no_browser } => {
                let mut cfg = EnsureAccessKeyConfig::from_env(chain_id);
                cfg.no_browser |= no_browser;
                let outcome = ensure_access_key(cfg).await?;
                let _ = foundry_common::sh_status!(
                    "Authorized key {} for wallet {} on chain {}",
                    outcome.key_address,
                    outcome.wallet_address,
                    outcome.chain_id,
                );
                Ok(())
            }
            Self::ImportAccessKey { account, access_key, authorization } => {
                let authorization =
                    decode_key_authorization::<SignedKeyAuthorization>(&authorization)?;
                let chain_id = authorization.chain_id;
                let key_address = access_key.address();
                let store = TempoAccountsStore::default_path()?;
                store.upsert_secp256k1_access_key(account, &access_key, &authorization)?;
                let _ = foundry_common::sh_status!(
                    "Imported access key {} for wallet {} on chain {} into {}",
                    key_address,
                    account,
                    chain_id,
                    store.path().display(),
                );
                Ok(())
            }
        }
    }
}
