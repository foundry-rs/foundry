use clap::{Args, Parser};
use eyre::Result;
use foundry_cli::json::print_json_success;
use foundry_common::{sh_println, shell};
use serde_json::json;

use super::{
    TouchIdSidecarState, ensure_touch_id_available, existing_keystore_path,
    remove_touch_id_sidecar, touch_id_sidecar_path, touch_id_sidecar_policy,
    touch_id_sidecar_state,
};

#[cfg(all(target_os = "macos", feature = "touch-id"))]
use alloy_signer_local::PrivateKeySigner;

#[cfg(all(target_os = "macos", feature = "touch-id"))]
use super::{ensure_touch_id_sidecar_available, password_or_prompt};

/// Arguments for `cast wallet touch-id`.
#[derive(Debug, Args)]
pub struct TouchIdArgs {
    #[command(subcommand)]
    command: TouchIdSubcommands,
}

impl TouchIdArgs {
    pub fn run(self) -> Result<()> {
        self.command.run()
    }
}

/// Touch ID lifecycle commands for encrypted keystores.
#[derive(Debug, Parser)]
enum TouchIdSubcommands {
    /// Enroll an existing keystore for Touch ID-assisted authentication.
    Enroll {
        /// The name of the keystore account.
        #[arg(value_name = "ACCOUNT_NAME")]
        account_name: String,

        /// The directory containing the keystore.
        #[arg(long, short)]
        keystore_dir: Option<String>,

        /// The keystore password in cleartext.
        #[arg(long, env = "CAST_UNSAFE_PASSWORD", value_name = "PASSWORD")]
        unsafe_password: Option<String>,
    },

    /// Report the persisted Touch ID enrollment state for a keystore.
    Status {
        /// The name of the keystore account.
        #[arg(value_name = "ACCOUNT_NAME")]
        account_name: String,

        /// The directory containing the keystore.
        #[arg(long, short)]
        keystore_dir: Option<String>,
    },

    /// Remove the persisted Touch ID enrollment for a keystore.
    Remove {
        /// The name of the keystore account.
        #[arg(value_name = "ACCOUNT_NAME")]
        account_name: String,

        /// The directory containing the keystore.
        #[arg(long, short)]
        keystore_dir: Option<String>,
    },
}

impl TouchIdSubcommands {
    fn run(self) -> Result<()> {
        match self {
            Self::Enroll { account_name, keystore_dir, unsafe_password } => {
                enroll(&account_name, keystore_dir, unsafe_password)
            }
            Self::Status { account_name, keystore_dir } => status(&account_name, keystore_dir),
            Self::Remove { account_name, keystore_dir } => remove(&account_name, keystore_dir),
        }
    }
}

fn status(account_name: &str, keystore_dir: Option<String>) -> Result<()> {
    let keystore_path = existing_keystore_path(account_name, keystore_dir)?;
    let sidecar = touch_id_sidecar_path(&keystore_path);

    match touch_id_sidecar_state(&sidecar)? {
        TouchIdSidecarState::Missing => print_status(
            json!({"account": account_name, "status": "not-enrolled"}),
            format!("Touch ID is not enrolled for keystore `{account_name}`."),
        ),
        TouchIdSidecarState::Recognized => {
            let policy = touch_id_sidecar_policy(&sidecar)?.as_str();
            print_status(
                json!({"account": account_name, "status": "enrolled", "policy": policy}),
                format!(
                    "Touch ID is enrolled for keystore `{account_name}` with `{policy}` policy."
                ),
            )
        }
        TouchIdSidecarState::Keystore => print_status(
            json!({"account": account_name, "status": "conflict"}),
            format!(
                "Touch ID status for keystore `{account_name}` is conflicted: {} is an existing keystore.",
                sidecar.display()
            ),
        ),
        TouchIdSidecarState::Unknown => print_status(
            json!({"account": account_name, "status": "unknown"}),
            format!(
                "Touch ID status for keystore `{account_name}` is unknown: {} is not a recognized Touch ID sidecar.",
                sidecar.display()
            ),
        ),
    }
}

fn remove(account_name: &str, keystore_dir: Option<String>) -> Result<()> {
    let keystore_path = existing_keystore_path(account_name, keystore_dir)?;
    let removed = remove_touch_id_sidecar(&keystore_path)?;
    let message = if removed {
        format!("Touch ID enrollment removed for keystore `{account_name}`.")
    } else {
        format!("Touch ID is not enrolled for keystore `{account_name}`.")
    };
    print_status(json!({"account": account_name, "removed": removed}), message)
}

#[cfg(all(target_os = "macos", feature = "touch-id"))]
fn enroll(
    account_name: &str,
    keystore_dir: Option<String>,
    unsafe_password: Option<String>,
) -> Result<()> {
    let keystore_path = existing_keystore_path(account_name, keystore_dir)?;
    ensure_touch_id_available(true)?;

    let sidecar = touch_id_sidecar_path(&keystore_path);
    let state = touch_id_sidecar_state(&sidecar)?;
    ensure_touch_id_sidecar_available(&keystore_path)?;
    let (reenrolled, policy) = match state {
        TouchIdSidecarState::Missing => (false, foundry_wallets::touch_id::Policy::default()),
        TouchIdSidecarState::Recognized => {
            (true, foundry_wallets::touch_id::policy(&keystore_path)?)
        }
        TouchIdSidecarState::Keystore | TouchIdSidecarState::Unknown => {
            eyre::bail!("Touch ID sidecar state changed during enrollment preflight");
        }
    };

    let password = password_or_prompt(unsafe_password, "Enter password: ")?;
    PrivateKeySigner::decrypt_keystore(&keystore_path, &password)
        .map_err(|_| eyre::eyre!("Invalid password - Touch ID enrollment cancelled"))?;

    foundry_wallets::touch_id::enroll(&keystore_path, &password, policy).map_err(|error| {
        let action = if reenrolled { "re-enrollment" } else { "enrollment" };
        eyre::eyre!("Touch ID {action} failed for keystore `{account_name}`: {error}")
    })?;

    let message = if reenrolled {
        format!("Touch ID re-enrolled for keystore `{account_name}`.")
    } else {
        format!("Touch ID enrolled for keystore `{account_name}`.")
    };
    print_status(
        json!({"account": account_name, "touch_id": true, "reenrolled": reenrolled}),
        message,
    )
}

#[cfg(not(all(target_os = "macos", feature = "touch-id")))]
fn enroll(
    account_name: &str,
    keystore_dir: Option<String>,
    _unsafe_password: Option<String>,
) -> Result<()> {
    existing_keystore_path(account_name, keystore_dir)?;
    ensure_touch_id_available(true)
}

fn print_status(value: serde_json::Value, message: String) -> Result<()> {
    if shell::is_json() { print_json_success(value) } else { sh_println!("{message}") }
}
