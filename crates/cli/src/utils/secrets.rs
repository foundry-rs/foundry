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
    Failed(String),
}

static STORE: LazyLock<Result<SecretStore, SecretStoreError>> =
    LazyLock::new(SecretStore::open_default);
static UNLOCK: Mutex<Unlock> = Mutex::new(Unlock::Locked);

/// Returns the stored secret `name`, unlocking the store on first use.
///
/// Returns `None`, without asking for a password, if no such secret is stored.
pub(crate) fn resolve_secret(name: &str) -> Result<Option<String>, String> {
    let store = STORE.as_ref().map_err(ToString::to_string)?;
    if !store.contains(name).map_err(|err| err.to_string())? {
        return Ok(None);
    }

    // Held while prompting, so concurrent lookups wait for one prompt.
    let mut unlock = UNLOCK.lock().map_err(|_| "secret store lock poisoned".to_string())?;
    if matches!(*unlock, Unlock::Locked) {
        *unlock = match password(store, name) {
            Ok(password) => Unlock::Unlocked(password),
            Err(err) => Unlock::Failed(err),
        };
    }
    match &*unlock {
        Unlock::Unlocked(password) => {
            store.get(name, password).map(Some).map_err(|err| err.to_string())
        }
        Unlock::Failed(err) => Err(err.clone()),
        Unlock::Locked => unreachable!(),
    }
}

fn password(store: &SecretStore, name: &str) -> Result<String, String> {
    if let Ok(password) = std::env::var(SECRETS_PASSWORD_ENV) {
        store.check_password(&password).map_err(|err| err.to_string())?;
        return Ok(password);
    }

    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Err(format!(
            "`{name}` is in the secret store, which is locked: set {SECRETS_PASSWORD_ENV} or run in a terminal"
        ));
    }

    for _ in 0..PROMPT_ATTEMPTS {
        let password = rpassword::prompt_password(format!(
            "Password for the Foundry secret store (to read `{name}`): "
        ))
        .map_err(|err| err.to_string())?;
        match store.check_password(&password) {
            Ok(()) => return Ok(password),
            Err(SecretStoreError::WrongPassword) => {
                let _ = sh_warn!("wrong password");
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    Err("wrong password for the secret store".to_string())
}
