use clap::Parser;
use ethers::{
    signers::{LocalWallet, Signer},
    types::Address,
};
use eyre::Result;

use serde::Serialize;

use super::wallet::WalletTrait;

#[derive(Parser, Debug, Clone, Serialize, Default)]
#[cfg_attr(not(doc), allow(missing_docs))]
#[cfg_attr(
    doc,
    doc = r#"
The wallet options can either be:
1. Ledger
2. Trezor
3. Mnemonics (via file path)
4. Keystores (via file path)
5. Private Keys (cleartext in CLI)
6. Private Keys (interactively via secure prompt)
"#
)]
pub struct MultiWallet {
    #[clap(
        long,
        short,
        help_heading = "WALLET OPTIONS - RAW",
        help = "Open an interactive prompt to enter your private key. Takes a value for the number of keys to enter",
        default_value = "0"
    )]
    pub interactives: u32,

    #[clap(
        long = "private-keys",
        help_heading = "WALLET OPTIONS - RAW",
        help = "Use the provided private key."
    )]
    pub private_keys: Option<Vec<String>>,

    #[clap(
        long = "private-key",
        help_heading = "WALLET OPTIONS - RAW",
        help = "Use the provided private key.",
        conflicts_with = "private-keys"
    )]
    pub private_key: Option<String>,

    #[clap(
        long = "mnemonic-paths",
        help_heading = "WALLET OPTIONS - RAW",
        help = "Use the mnemonic file at the specified path."
    )]
    pub mnemonic_paths: Option<Vec<String>>,

    #[clap(
        long = "mnemonic-indexes",
        help_heading = "WALLET OPTIONS - RAW",
        help = "Use the private key from the given mnemonic index. Used with --mnemonic-path.",
        default_value = "0"
    )]
    pub mnemonic_indexes: Option<Vec<u32>>,

    #[clap(
        env = "ETH_KEYSTORE",
        long = "keystores",
        help_heading = "WALLET OPTIONS - KEYSTORE",
        help = "Use the keystore in the given folder or file."
    )]
    pub keystore_paths: Option<Vec<String>>,

    #[clap(
        long = "password",
        help_heading = "WALLET OPTIONS - KEYSTORE",
        help = "The keystore password. Used with --keystore.",
        requires = "keystore-paths"
    )]
    pub keystore_passwords: Option<Vec<String>>,

    #[clap(
        short,
        long = "ledger",
        help_heading = "WALLET OPTIONS - HARDWARE WALLET",
        help = "Use a Ledger hardware wallet."
    )]
    pub ledger: bool,

    #[clap(
        short,
        long = "trezor",
        help_heading = "WALLET OPTIONS - HARDWARE WALLET",
        help = "Use a Trezor hardware wallet."
    )]
    pub trezor: bool,

    #[clap(
        long = "hd-paths",
        help_heading = "WALLET OPTIONS - HARDWARE WALLET",
        help = "The derivation path to use with hardware wallets."
    )]
    pub hd_path: Option<String>,

    #[clap(
        env = "ETH_FROM",
        short = 'a',
        long = "froms",
        help_heading = "WALLET OPTIONS - REMOTE",
        help = "The sender account."
    )]
    pub froms: Option<Vec<Address>>,
}

impl WalletTrait for MultiWallet {}

impl MultiWallet {
    // TODO: Add trezor and ledger support (supported in multiwallet, just need to
    // add derivation + SignerMiddleware creation logic)
    // foundry/cli/src/opts/mod.rs:110
    pub fn all(&self, chain: u64) -> Result<Vec<LocalWallet>> {
        let mut local_wallets = vec![];
        if let Some(wallets) = self.private_keys()? {
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet.with_chain_id(chain)));
        }

        if let Some(wallets) = self.interactives()? {
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet.with_chain_id(chain)));
        }

        if let Some(wallets) = self.mnemonics()? {
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet.with_chain_id(chain)));
        }

        if let Some(wallets) = self.keystores()? {
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet.with_chain_id(chain)));
        }
        Ok(local_wallets)
    }

    pub fn interactives(&self) -> Result<Option<Vec<LocalWallet>>> {
        if self.interactives != 0 {
            let mut wallets = vec![];
            for _ in 0..self.interactives {
                wallets.push(self.get_from_interactive()?);
            }
            return Ok(Some(wallets))
        }
        Ok(None)
    }

    pub fn private_keys(&self) -> Result<Option<Vec<LocalWallet>>> {
        if let Some(private_keys) = &self.private_keys {
            let mut wallets = vec![];
            for private_key in private_keys.iter() {
                wallets.push(self.get_from_private_key(private_key)?);
            }
            return Ok(Some(wallets))
        }
        Ok(None)
    }

    pub fn keystores(&self) -> Result<Option<Vec<LocalWallet>>> {
        if let Some(keystore_paths) = &self.keystore_paths {
            let mut wallets = vec![];

            let mut passwords: Vec<Option<String>> = self
                .keystore_passwords
                .clone()
                .unwrap_or_default()
                .iter()
                .map(|pw| Some(pw.clone()))
                .collect();

            if passwords.is_empty() {
                passwords = vec![None; keystore_paths.len()]
            } else if passwords.len() != keystore_paths.len() {
                eyre::bail!("Keystore passwords don't have the same length as keystore paths.");
            }

            for (path, password) in keystore_paths.iter().zip(passwords) {
                wallets.push(self.get_from_keystore(Some(path), password.as_ref())?.unwrap());
            }
            return Ok(Some(wallets))
        }
        Ok(None)
    }

    pub fn mnemonics(&self) -> Result<Option<Vec<LocalWallet>>> {
        if let (Some(mnemonic_paths), Some(mnemonic_indexes)) =
            (self.mnemonic_paths.as_ref(), self.mnemonic_indexes.as_ref())
        {
            let mut wallets = vec![];
            for (path, mnemonic_index) in mnemonic_paths.iter().zip(mnemonic_indexes) {
                wallets.push(self.get_from_mnemonic(path, *mnemonic_index)?)
            }
            return Ok(Some(wallets))
        }
        Ok(None)
    }
}
