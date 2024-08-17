use crate::{raw_wallet::RawWalletOpts, utils, wallet_signer::WalletSigner};
use alloy_primitives::Address;
use clap::Parser;
use eyre::Result;
use serde::Serialize;

/// The wallet options can either be:
/// 1. Raw (via private key / mnemonic file, see `RawWallet`)
/// 2. Ledger
/// 3. Trezor
/// 4. Keystore (via file path)
/// 5. AWS KMS
/// 6. Google Cloud KMS
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[command(next_help_heading = "Wallet options", about = None, long_about = None)]
pub struct WalletOpts {
    /// The sender account.
    #[arg(
        long,
        short,
        value_name = "ADDRESS",
        help_heading = "Wallet options - raw",
        env = "ETH_FROM"
    )]
    pub from: Option<Address>,

    #[command(flatten)]
    pub raw: RawWalletOpts,

    /// Use the keystore in the given folder or file.
    #[arg(
        long = "keystore",
        help_heading = "Wallet options - keystore",
        value_name = "PATH",
        env = "ETH_KEYSTORE"
    )]
    pub keystore_path: Option<String>,

    /// Use a keystore from the default keystores folder (~/.foundry/keystores) by its filename
    #[arg(
        long = "account",
        help_heading = "Wallet options - keystore",
        value_name = "ACCOUNT_NAME",
        env = "ETH_KEYSTORE_ACCOUNT",
        conflicts_with = "keystore_path"
    )]
    pub keystore_account_name: Option<String>,

    /// The keystore password.
    ///
    /// Used with --keystore.
    #[arg(
        long = "password",
        help_heading = "Wallet options - keystore",
        requires = "keystore_path",
        value_name = "PASSWORD"
    )]
    pub keystore_password: Option<String>,

    /// The keystore password file path.
    ///
    /// Used with --keystore.
    #[arg(
        long = "password-file",
        help_heading = "Wallet options - keystore",
        requires = "keystore_path",
        value_name = "PASSWORD_FILE",
        env = "ETH_PASSWORD"
    )]
    pub keystore_password_file: Option<String>,

    /// Use a Ledger hardware wallet.
    #[arg(long, short, help_heading = "Wallet options - hardware wallet")]
    pub ledger: bool,

    /// Use a Trezor hardware wallet.
    #[arg(long, short, help_heading = "Wallet options - hardware wallet")]
    pub trezor: bool,

    /// Use AWS Key Management Service.
    #[arg(long, help_heading = "Wallet options - remote", hide = !cfg!(feature = "aws-kms"))]
    pub aws: bool,

    /// Use Google Cloud Key Management Service.
    #[arg(long, help_heading = "Wallet options - remote", hide = !cfg!(feature = "gcp-kms"))]
    pub gcp: bool,
}

impl WalletOpts {
    pub async fn signer(&self) -> Result<WalletSigner> {
        trace!("start finding signer");

        let signer = if self.ledger {
            utils::create_ledger_signer(self.raw.hd_path.as_deref(), self.raw.mnemonic_index)
                .await?
        } else if self.trezor {
            utils::create_trezor_signer(self.raw.hd_path.as_deref(), self.raw.mnemonic_index)
                .await?
        } else if self.aws {
            let key_id = std::env::var("AWS_KMS_KEY_ID")?;
            WalletSigner::from_aws(key_id).await?
        } else if self.gcp {
            let project_id = std::env::var("GCP_PROJECT_ID")?;
            let location = std::env::var("GCP_LOCATION")?;
            let keyring = std::env::var("GCP_KEYRING")?;
            let key_name = std::env::var("GCP_KEY_NAME")?;
            let key_version = std::env::var("GCP_KEY_VERSION")?.parse()?;
            WalletSigner::from_gcp(project_id, location, keyring, key_name, key_version).await?
        } else if let Some(raw_wallet) = self.raw.signer()? {
            raw_wallet
        } else if let Some(path) = utils::maybe_get_keystore_path(
            self.keystore_path.as_deref(),
            self.keystore_account_name.as_deref(),
        )? {
            let (maybe_signer, maybe_pending) = utils::create_keystore_signer(
                &path,
                self.keystore_password.as_deref(),
                self.keystore_password_file.as_deref(),
            )?;
            if let Some(pending) = maybe_pending {
                pending.unlock()?
            } else if let Some(signer) = maybe_signer {
                signer
            } else {
                unreachable!()
            }
        } else {
            eyre::bail!(
                "\
Error accessing local wallet. Did you set a private key, mnemonic or keystore?
Run `cast send --help` or `forge create --help` and use the corresponding CLI
flag to set your key via:
--private-key, --mnemonic-path, --aws, --gcp, --interactive, --trezor or --ledger.
Alternatively, if you're using a local node with unlocked accounts,
use the --unlocked flag and either set the `ETH_FROM` environment variable to the address
of the unlocked account you want to use, or provide the --from flag with the address directly."
            )
        };

        Ok(signer)
    }
}

impl From<RawWalletOpts> for WalletOpts {
    fn from(options: RawWalletOpts) -> Self {
        Self { raw: options, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use alloy_signer::Signer;
    use std::{path::Path, str::FromStr};

    use super::*;

    #[tokio::test]
    async fn find_keystore() {
        let keystore =
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../cast/tests/fixtures/keystore"));
        let keystore_file = keystore
            .join("UTC--2022-12-20T10-30-43.591916000Z--ec554aeafe75601aaab43bd4621a22284db566c2");
        let password_file = keystore.join("password-ec554");
        let wallet: WalletOpts = WalletOpts::parse_from([
            "foundry-cli",
            "--from",
            "560d246fcddc9ea98a8b032c9a2f474efb493c28",
            "--keystore",
            keystore_file.to_str().unwrap(),
            "--password-file",
            password_file.to_str().unwrap(),
        ]);
        let signer = wallet.signer().await.unwrap();
        assert_eq!(
            signer.address(),
            Address::from_str("ec554aeafe75601aaab43bd4621a22284db566c2").unwrap()
        );
    }

    #[tokio::test]
    async fn illformed_private_key_generates_user_friendly_error() {
        let wallet = WalletOpts {
            raw: RawWalletOpts {
                interactive: false,
                private_key: Some("123".to_string()),
                mnemonic: None,
                mnemonic_passphrase: None,
                hd_path: None,
                mnemonic_index: 0,
            },
            from: None,
            keystore_path: None,
            keystore_account_name: None,
            keystore_password: None,
            keystore_password_file: None,
            ledger: false,
            trezor: false,
            aws: false,
            gcp: false,
        };
        match wallet.signer().await {
            Ok(_) => {
                panic!("illformed private key shouldn't decode")
            }
            Err(x) => {
                assert!(
                    x.to_string().contains("Failed to decode private key"),
                    "Error message is not user-friendly"
                );
            }
        }
    }
}
