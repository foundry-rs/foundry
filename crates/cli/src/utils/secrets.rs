//! Resolves `${NAME}` config placeholders from the encrypted secret store.

use foundry_common::sh_warn;
use foundry_config::secrets::{SecretStore, SecretStoreError};
use std::{
    io::IsTerminal,
    sync::{LazyLock, Mutex},
};

/// Environment variable that holds the secret store password, for non-interactive use.
pub const SECRETS_PASSWORD_ENV: &str = "FOUNDRY_SECRETS_PASSWORD";

/// Password prompts before giving up.
const PROMPT_ATTEMPTS: usize = 3;

#[derive(Default)]
enum Unlock {
    #[default]
    Locked,
    Unlocked(String),
    /// No password was available, or it was wrong. Do not ask again in this process.
    Failed,
}

static STORE: LazyLock<Option<SecretStore>> = LazyLock::new(|| SecretStore::open_default().ok());
static UNLOCK: Mutex<Unlock> = Mutex::new(Unlock::Locked);

/// Returns the stored secret `name`, unlocking the store on first use.
///
/// Returns `None`, without asking for a password, if no such secret is stored.
pub(crate) fn resolve_secret(name: &str) -> Option<String> {
    let store = STORE.as_ref()?;
    if !store.contains(name) {
        return None;
    }

    // Held while prompting, so concurrent lookups wait for one prompt.
    let mut unlock = UNLOCK.lock().ok()?;
    if matches!(*unlock, Unlock::Locked) {
        *unlock = match password(store, name) {
            Some(password) => Unlock::Unlocked(password),
            None => Unlock::Failed,
        };
    }
    let Unlock::Unlocked(password) = &*unlock else { return None };

    match store.get(name, password) {
        Ok(value) => Some(value),
        Err(err) => {
            let _ = sh_warn!("could not read secret `{name}`: {err}");
            None
        }
    }
}

fn password(store: &SecretStore, name: &str) -> Option<String> {
    if let Ok(password) = std::env::var(SECRETS_PASSWORD_ENV) {
        return match store.check_password(&password) {
            Ok(()) => Some(password),
            Err(err) => {
                let _ = sh_warn!(
                    "could not unlock the secret store with {SECRETS_PASSWORD_ENV}: {err}"
                );
                None
            }
        };
    }

    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        let _ = sh_warn!(
            "`{name}` is in the secret store, which is locked: set {SECRETS_PASSWORD_ENV} or run in a terminal"
        );
        return None;
    }

    for _ in 0..PROMPT_ATTEMPTS {
        let password = rpassword::prompt_password(format!(
            "Password for the Foundry secret store (to read `{name}`): "
        ))
        .ok()?;
        match store.check_password(&password) {
            Ok(()) => return Some(password),
            Err(SecretStoreError::WrongPassword) => {
                let _ = sh_warn!("wrong password");
            }
            Err(err) => {
                let _ = sh_warn!("could not unlock the secret store: {err}");
                return None;
            }
        }
    }
    None
}
