use clap::Parser;
use eyre::{Result, bail};
use foundry_common::sh_println;
use foundry_config::secrets::SecretStore;
use std::io::{IsTerminal, Read};

/// Environment variable that holds the secret store password, for non-interactive use.
const SECRETS_PASSWORD_ENV: &str = "FOUNDRY_SECRETS_PASSWORD";

/// Manage encrypted secrets, such as RPC URLs and API keys.
///
/// A `${NAME}` placeholder in `foundry.toml` (for example in `[rpc_endpoints]` or `[etherscan]`)
/// resolves from the secret store when `NAME` is not set in the environment. Secrets are encrypted
/// with one password, in `~/.foundry/secrets` (or `$FOUNDRY_SECRETS_DIR`).
#[derive(Clone, Debug, Parser)]
pub enum SecretSubcommand {
    /// Store a secret, replacing any previous value.
    ///
    /// The value is read from a hidden prompt, or from stdin when it is not a terminal.
    #[command(visible_alias = "s")]
    Set {
        /// The secret name, as used in `${NAME}`.
        name: String,
    },

    /// Print a secret.
    Get {
        /// The secret name.
        name: String,
    },

    /// List the names of the stored secrets.
    #[command(visible_alias = "ls")]
    List,

    /// Delete a secret.
    #[command(visible_aliases = ["rm", "delete"])]
    Remove {
        /// The secret name.
        name: String,
    },

    /// Print the secret store directory.
    Path,
}

impl SecretSubcommand {
    pub fn run(self) -> Result<()> {
        let store = SecretStore::open_default()?;
        match self {
            Self::Set { name } => {
                let value = read_value(&name)?;
                let password = password(&store, true)?;
                store.set(&name, &value, &password)?;
                sh_println!("Stored secret `{name}` in {}", store.dir().display())?;
            }
            Self::Get { name } => {
                if !store.contains(&name) {
                    bail!("secret `{name}` not found");
                }
                let password = password(&store, false)?;
                sh_println!("{}", store.get(&name, &password)?)?;
            }
            Self::List => {
                for name in store.names()? {
                    sh_println!("{name}")?;
                }
            }
            Self::Remove { name } => {
                store.remove(&name)?;
                sh_println!("Removed secret `{name}`")?;
            }
            Self::Path => sh_println!("{}", store.dir().display())?,
        }
        Ok(())
    }
}

/// Reads the secret value from a hidden prompt, or from stdin when it is not a terminal.
fn read_value(name: &str) -> Result<String> {
    let value = if std::io::stdin().is_terminal() {
        rpassword::prompt_password(format!("Value for `{name}`: "))?
    } else {
        let mut value = String::new();
        std::io::stdin().read_to_string(&mut value)?;
        value.trim_end_matches(['\r', '\n']).to_string()
    };
    if value.is_empty() {
        bail!("empty value for `{name}`");
    }
    Ok(value)
}

/// Returns the store password from `FOUNDRY_SECRETS_PASSWORD` or a hidden prompt. A new store asks
/// for the password twice.
fn password(store: &SecretStore, setting: bool) -> Result<String> {
    if let Ok(password) = std::env::var(SECRETS_PASSWORD_ENV) {
        return Ok(password);
    }
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to ask for the secret store password: set {SECRETS_PASSWORD_ENV}");
    }
    let password = rpassword::prompt_password("Secret store password: ")?;
    if setting && store.names()?.is_empty() {
        let confirm = rpassword::prompt_password("Confirm the new secret store password: ")?;
        if confirm != password {
            bail!("passwords do not match");
        }
    }
    Ok(password)
}
