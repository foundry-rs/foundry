use clap::Parser;
use serde::Serialize;

/// A wrapper for the raw data options for `Wallet`, extracted to also be used standalone.
/// The raw wallet options can either be:
/// 1. Private Key (cleartext in CLI)
/// 2. Private Key (interactively via secure prompt)
/// 3. Mnemonic (via file path)
#[derive(Clone, Debug, Default, Serialize, Parser)]
#[clap(next_help_heading = "Wallet options - raw", about = None, long_about = None)]
pub struct RawWallet {
    /// Open an interactive prompt to enter your private key.
    #[clap(long, short)]
    pub interactive: bool,

    /// Use the provided private key.
    #[clap(long, value_name = "RAW_PRIVATE_KEY")]
    pub private_key: Option<String>,

    /// Use the mnemonic phrase of mnemonic file at the specified path.
    #[clap(long, alias = "mnemonic-path")]
    pub mnemonic: Option<String>,

    /// Use a BIP39 passphrase for the mnemonic.
    #[clap(long, value_name = "PASSPHRASE")]
    pub mnemonic_passphrase: Option<String>,

    /// The wallet derivation path.
    ///
    /// Works with both --mnemonic-path and hardware wallets.
    #[clap(long = "mnemonic-derivation-path", alias = "hd-path", value_name = "PATH")]
    pub hd_path: Option<String>,

    /// Use the private key from the given mnemonic index.
    ///
    /// Used with --mnemonic-path.
    #[clap(long, conflicts_with = "hd_path", default_value_t = 0, value_name = "INDEX")]
    pub mnemonic_index: u32,
}
