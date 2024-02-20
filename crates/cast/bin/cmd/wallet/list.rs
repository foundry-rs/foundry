use clap::Parser;
use eyre::Result;

use foundry_common::{fs, types::ToAlloy};
use foundry_config::Config;
use foundry_wallets::{multi_wallet::MultiWalletOptsBuilder, WalletSigner};

/// CLI arguments for `cast wallet list`.
#[derive(Clone, Debug, Parser)]
pub struct ListArgs {
    /// List all the accounts in the keystore directory.
    /// Default keystore directory is used if no path provided.
    #[clap(long, default_missing_value = "", num_args(0..=1), help_heading = "List local accounts")]
    dir: Option<String>,

    /// List accounts from a Ledger hardware wallet.
    #[clap(long, short, help_heading = "List Ledger hardware wallet accounts")]
    ledger: bool,

    /// List accounts from a Trezor hardware wallet.
    #[clap(long, short, help_heading = "List Trezor hardware wallet accounts")]
    trezor: bool,

    /// List accounts from AWS KMS.
    #[clap(long, help_heading = "List AWS KMS account")]
    aws: bool,

    /// List all configured accounts.
    #[clap(long, help_heading = "List all accounts")]
    all: bool,
}

impl ListArgs {
    pub async fn run(self) -> Result<()> {
        // list local accounts as files in keystore dir, no need to unlock / provide password
        if self.dir.is_some() || self.all || !self.ledger && !self.trezor && !self.aws {
            self.list_local_senders().await?;
        }

        // Create options for multi wallet - ledger, trezor and AWS
        let list_opts = MultiWalletOptsBuilder::default()
            .ledger(self.ledger || self.all)
            .mnemonic_indexes(Some(vec![0]))
            .trezor(self.trezor || self.all)
            .aws(self.aws || self.all)
            .froms(None)
            .interactives(0)
            .private_keys(None)
            .private_key(None)
            .mnemonics(None)
            .mnemonic_passphrases(None)
            .hd_paths(None)
            .keystore_paths(None)
            .keystore_account_names(None)
            .keystore_passwords(None)
            .keystore_password_files(None)
            .build()
            .expect("build multi wallet");

        // max number of senders to be shown for ledger and trezor signers
        let max_senders = 3;

        // List ledger accounts
        match list_opts.ledgers().await {
            Ok(signers) => {
                self.list_senders(signers.unwrap_or_default(), max_senders, "Ledger").await?
            }
            Err(e) => {
                if !self.all {
                    println!("{}", e)
                }
            }
        }

        // List Trezor accounts
        match list_opts.trezors().await {
            Ok(signers) => {
                self.list_senders(signers.unwrap_or_default(), max_senders, "Trezor").await?
            }
            Err(e) => {
                if !self.all {
                    println!("{}", e)
                }
            }
        }

        // List AWS accounts
        match list_opts.aws_signers().await {
            Ok(signers) => {
                self.list_senders(signers.unwrap_or_default(), max_senders, "AWS").await?
            }
            Err(e) => {
                if !self.all {
                    println!("{}", e)
                }
            }
        }

        Ok(())
    }

    async fn list_local_senders(&self) -> Result<()> {
        let keystore_path = self.dir.clone().unwrap_or_default();
        let keystore_dir = if keystore_path.is_empty() {
            let default_dir = Config::foundry_keystores_dir()
                .ok_or_else(|| eyre::eyre!("Could not find the default keystore directory."))?;
            // Create the keystore directory if it doesn't exist
            fs::create_dir_all(&default_dir)?;
            default_dir
        } else {
            dunce::canonicalize(keystore_path)?
        };

        match std::fs::read_dir(keystore_dir) {
            Ok(entries) => {
                entries.flatten().for_each(|entry| {
                    let path = entry.path();
                    if path.is_file() && path.extension().is_none() {
                        if let Some(file_name) = path.file_name() {
                            if let Some(name) = file_name.to_str() {
                                println!("{} (Local)", name);
                            }
                        }
                    }
                });
            }
            Err(e) => {
                if !self.all {
                    println!("{}", e)
                }
            }
        }

        Ok(())
    }

    async fn list_senders(
        &self,
        signers: Vec<WalletSigner>,
        max_senders: usize,
        label: &str,
    ) -> eyre::Result<()> {
        for signer in signers.iter() {
            match signer.available_senders(max_senders).await {
                Ok(senders) => {
                    senders.iter().for_each(|sender| println!("{} ({})", sender.to_alloy(), label));
                }
                Err(e) => {
                    if !self.all {
                        println!("{}", e)
                    }
                }
            }
        }

        Ok(())
    }
}
