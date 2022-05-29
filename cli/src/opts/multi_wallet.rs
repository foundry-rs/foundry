use std::collections::{HashMap, HashSet};

use clap::Parser;
use ethers::{prelude::Signer, signers::LocalWallet, types::Address};
use eyre::Result;

use foundry_config::Config;
use serde::Serialize;

use super::wallet::WalletTrait;

macro_rules! get_wallets {
    ($id:ident, [ $($wallets:expr),+ ], $call:expr) => {
        $(
            if let Some($id) = $wallets {
                $call;
            }
        )+
    };
}

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
    pub fn find_all(
        &self,
        chain: u64,
        mut addresses: HashSet<Address>,
    ) -> Result<HashMap<Address, LocalWallet>> {
        println!("\n###\nFinding wallets for all the necessary addresses...");

        let mut local_wallets = HashMap::new();
        let mut unused_wallets = vec![];

        get_wallets!(
            wallets,
            [self.private_keys()?, self.interactives()?, self.mnemonics()?, self.keystores()?],
            for wallet in wallets.into_iter() {
                let address = &wallet.address();

                if addresses.contains(address) {
                    addresses.remove(address);

                    local_wallets.insert(*address, wallet.with_chain_id(chain));

                    if addresses.is_empty() {
                        return Ok(local_wallets)
                    }
                } else {
                    // Just to show on error.
                    unused_wallets.push(wallet);
                }
            }
        );

        let mut error_msg = "".to_string();

        // This is an actual used address
        if addresses.contains(&Config::DEFAULT_SENDER) {
            error_msg += "\nYou seem to be using Foundry's default sender. Be sure to set your own --sender.\n";
        }

        unused_wallets.extend(local_wallets.into_values());
        eyre::bail!(
            "{}No associated wallet for addresses: {:?}. Unlocked wallets: {:?}",
            error_msg,
            addresses,
            unused_wallets.into_iter().map(|wallet| wallet.address()).collect::<Vec<Address>>()
        )
    }

    pub fn all(&self, chain: u64) -> Result<Vec<LocalWallet>> {
        let mut local_wallets = vec![];

        get_wallets!(
            wallets,
            [self.private_keys()?, self.interactives()?, self.mnemonics()?, self.keystores()?],
            wallets.into_iter().for_each(|wallet| local_wallets.push(wallet.with_chain_id(chain)))
        );

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
