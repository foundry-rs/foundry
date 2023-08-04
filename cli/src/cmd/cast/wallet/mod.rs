//! cast wallet subcommand

pub mod vanity;

use crate::{
    cmd::{cast::wallet::vanity::VanityArgs, Cmd},
    opts::Wallet,
};
use cast::SimpleCast;
use clap::Parser;
use ethers::{
    core::rand::thread_rng,
    signers::{LocalWallet, Signer},
    types::{transaction::eip712::TypedData, Address, Signature, H256},
    utils::keccak256,
};
use eyre::Context;

/// CLI arguments for `cast wallet`.
#[derive(Debug, Parser)]
pub enum WalletSubcommands {
    /// Create a new random keypair.
    #[clap(visible_alias = "n")]
    New {
        /// If provided, then keypair will be written to an encrypted JSON keystore.
        path: Option<String>,

        /// Triggers a hidden password prompt for the JSON keystore.
        ///
        /// Deprecated: prompting for a hidden password is now the default.
        #[clap(long, short, requires = "path", conflicts_with = "unsafe_password")]
        password: bool,

        /// Password for the JSON keystore in cleartext.
        ///
        /// This is UNSAFE to use and we recommend using the --password.
        #[clap(long, requires = "path", env = "CAST_PASSWORD", value_name = "PASSWORD")]
        unsafe_password: Option<String>,
    },

    /// Generate a vanity address.
    #[clap(visible_alias = "va")]
    Vanity(VanityArgs),

    /// Convert a private key to an address.
    #[clap(visible_aliases = &["a", "addr"])]
    Address {
        /// If provided, the address will be derived from the specified private key.
        #[clap(
            value_name = "PRIVATE_KEY",
            value_parser = foundry_common::clap_helpers::strip_0x_prefix,
        )]
        private_key_override: Option<String>,

        #[clap(flatten)]
        wallet: Wallet,
    },

    /// Sign a message or typed data.
    #[clap(visible_alias = "s")]
    Sign {
        /// The message or typed data to sign.
        ///
        /// Messages starting with 0x are expected to be hex encoded,
        /// which get decoded before being signed.
        /// By default, the message will be prefixed with the Ethereum Signed Message header and
        /// hashed before signing.
        ///
        /// Use --raw flag to denote the message as a string of a 256-bit hash.
        /// The message will not be hashed before signing. This flag is unaffected by the --data
        /// and --headerless flags.
        ///
        /// Use --headerless flag to denote the message to be signed without the Ethereum Signed
        /// Message header. The message will not be hashed using the EIP712 specification.
        ///
        /// Typed data can be provided as a json string or a file name.
        /// Use --data flag to denote the message as a string of typed data.
        /// Use --data --from-file to denote the message as a file name containing typed data.
        /// The data will be combined and hashed using the EIP712 specification before signing.
        /// The data should be formatted as JSON.
        message: String,

        /// If provided, the message will be treated as a 256-bit hash, and will not be hashed
        /// again.
        #[clap(long)]
        raw: bool,

        /// If provided, the message will not be hashed with the Ethereum Signed Message header.
        #[clap(long)]
        headerless: bool,

        /// If provided, the message will be treated as typed data.
        #[clap(long)]
        data: bool,

        /// If provided, the message will be treated as a file name containing typed data. Requires
        /// --data.
        #[clap(long, requires = "data")]
        from_file: bool,

        #[clap(flatten)]
        wallet: Wallet,
    },

    /// Verify the signature of a message.
    #[clap(visible_alias = "v")]
    Verify {
        /// The original message.
        message: String,

        /// The signature to verify.
        signature: Signature,

        /// The address of the message signer.
        #[clap(long, short)]
        address: Address,

        /// If provided, the message will be treated as a 256-bit hash, and will not be hashed when
        /// verifying the signature.
        #[clap(long)]
        raw: bool,
    },
}

impl WalletSubcommands {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            WalletSubcommands::New { path, unsafe_password, .. } => {
                let mut rng = thread_rng();

                if let Some(path) = path {
                    let path = dunce::canonicalize(path)?;
                    if !path.is_dir() {
                        // we require path to be an existing directory
                        eyre::bail!("`{}` is not a directory", path.display());
                    }

                    let password = if let Some(password) = unsafe_password {
                        password
                    } else {
                        // if no --unsafe-password was provided read via stdin
                        rpassword::prompt_password("Enter secret: ")?
                    };

                    let (wallet, uuid) =
                        LocalWallet::new_keystore(&path, &mut rng, password, None)?;

                    println!("Created new encrypted keystore file: {}", path.join(uuid).display());
                    println!("Address: {}", SimpleCast::to_checksum_address(&wallet.address()));
                } else {
                    let wallet = LocalWallet::new(&mut rng);
                    println!("Successfully created new keypair.");
                    println!("Address:     {}", SimpleCast::to_checksum_address(&wallet.address()));
                    println!("Private key: 0x{}", hex::encode(wallet.signer().to_bytes()));
                }
            }
            WalletSubcommands::Vanity(cmd) => {
                cmd.run()?;
            }
            WalletSubcommands::Address { wallet, private_key_override } => {
                let wallet = private_key_override
                    .map(|pk| Wallet { private_key: Some(pk), ..Default::default() })
                    .unwrap_or(wallet)
                    .signer(0)
                    .await?;
                let addr = wallet.address();
                println!("{}", SimpleCast::to_checksum_address(&addr));
            }
            WalletSubcommands::Sign { message, raw, headerless, data, from_file, wallet } => {
                let wallet = wallet.signer(0).await?;
                let sig = if raw {
                    wallet.sign_hash(H256::from_slice(&hex::decode(&message)?)).await?
                } else if headerless {
                    wallet.sign_hash(H256(keccak256(Self::hex_str_to_bytes(&message)?))).await?
                } else if data {
                    let typed_data: TypedData = if from_file {
                        // data is a file name, read json from file
                        foundry_common::fs::read_json_file(message.as_ref())?
                    } else {
                        // data is a json string
                        serde_json::from_str(&message)?
                    };
                    wallet.sign_typed_data(&typed_data).await?
                } else {
                    wallet.sign_message(Self::hex_str_to_bytes(&message)?).await?
                };
                println!("0x{sig}");
            }
            WalletSubcommands::Verify { message, signature, address, raw } => {
                let result = if raw {
                    signature.verify(H256::from_slice(&hex::decode(&message)?), address)
                } else {
                    signature.verify(Self::hex_str_to_bytes(&message)?, address)
                };
                match result {
                    Ok(_) => {
                        println!("Validation succeeded. Address {address} signed this message.")
                    }
                    Err(_) => {
                        println!("Validation failed. Address {address} did not sign this message.")
                    }
                }
            }
        };

        Ok(())
    }

    fn hex_str_to_bytes(s: &str) -> eyre::Result<Vec<u8>> {
        Ok(match s.strip_prefix("0x") {
            Some(data) => hex::decode(data).wrap_err("Could not decode 0x-prefixed string.")?,
            None => s.as_bytes().to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_wallet_sign_message() {
        let args = WalletSubcommands::parse_from(["foundry-cli", "sign", "deadbeef"]);
        match args {
            WalletSubcommands::Sign { message, raw, headerless, data, from_file, .. } => {
                assert_eq!(message, "deadbeef".to_string());
                assert!(!raw);
                assert!(!headerless);
                assert!(!data);
                assert!(!from_file);
            }
            _ => panic!("expected WalletSubcommands::Sign"),
        }
    }

    #[test]
    fn can_parse_wallet_sign_hex_message() {
        let args = WalletSubcommands::parse_from(["foundry-cli", "sign", "0xdeadbeef"]);
        match args {
            WalletSubcommands::Sign { message, raw, headerless, data, from_file, .. } => {
                assert_eq!(message, "0xdeadbeef".to_string());
                assert!(!raw);
                assert!(!headerless);
                assert!(!data);
                assert!(!from_file);
            }
            _ => panic!("expected WalletSubcommands::Sign"),
        }
    }

    #[test]
    fn can_parse_wallet_sign_raw_message() {
        let args = WalletSubcommands::parse_from(["foundry-cli", "sign", "--raw", "deadbeef"]);
        match args {
            WalletSubcommands::Sign { message, raw, headerless, data, from_file, .. } => {
                assert_eq!(message, "deadbeef".to_string());
                assert!(raw);
                assert!(!headerless);
                assert!(!data);
                assert!(!from_file);
            }
            _ => panic!("expected WalletSubcommands::Sign"),
        }
    }

    #[test]
    fn can_parse_wallet_sign_headerless_message() {
        let args =
            WalletSubcommands::parse_from(["foundry-cli", "sign", "--headerless", "deadbeef"]);
        match args {
            WalletSubcommands::Sign { message, raw, headerless, data, from_file, .. } => {
                assert_eq!(message, "deadbeef".to_string());
                assert!(!raw);
                assert!(headerless);
                assert!(!data);
                assert!(!from_file);
            }
            _ => panic!("expected WalletSubcommands::Sign"),
        }
    }

    #[test]
    fn can_parse_wallet_sign_data() {
        let args = WalletSubcommands::parse_from(["foundry-cli", "sign", "--data", "{ ... }"]);
        match args {
            WalletSubcommands::Sign { message, raw, headerless, data, from_file, .. } => {
                assert_eq!(message, "{ ... }".to_string());
                assert!(!raw);
                assert!(!headerless);
                assert!(data);
                assert!(!from_file);
            }
            _ => panic!("expected WalletSubcommands::Sign"),
        }
    }

    #[test]
    fn can_parse_wallet_sign_data_file() {
        let args = WalletSubcommands::parse_from([
            "foundry-cli",
            "sign",
            "--data",
            "--from-file",
            "tests/data/typed_data.json",
        ]);
        match args {
            WalletSubcommands::Sign { message, raw, headerless, data, from_file, .. } => {
                assert_eq!(message, "tests/data/typed_data.json".to_string());
                assert!(!raw);
                assert!(!headerless);
                assert!(data);
                assert!(from_file);
            }
            _ => panic!("expected WalletSubcommands::Sign"),
        }
    }
}
