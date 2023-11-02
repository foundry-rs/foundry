use alloy_primitives::Address;
use clap::Parser;
use ethers::{
    core::rand::thread_rng,
    signers::{LocalWallet, Signer},
    types::{transaction::eip712::TypedData, Signature},
};
use eyre::{Context, Result};
use foundry_cli::opts::{RawWallet, Wallet};
use foundry_common::fs;
use foundry_config::Config;
use foundry_utils::types::{ToAlloy, ToEthers};
use std::path::Path;
use serde_json::json;
use yansi::Paint;

pub mod vanity;
use vanity::VanityArgs;

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

        /// Number wallet generation
        #[clap(long, short, default_value = "1")]
        number: u32,

        /// Output generated wallets as JSON.
        #[clap(long, short, default_value = "false")]
        json: bool,
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
        /// The message will be prefixed with the Ethereum Signed Message header and hashed before
        /// signing.
        ///
        /// Typed data can be provided as a json string or a file name.
        /// Use --data flag to denote the message is a string of typed data.
        /// Use --data --from-file to denote the message is a file name containing typed data.
        /// The data will be combined and hashed using the EIP712 specification before signing.
        /// The data should be formatted as JSON.
        message: String,

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
    },
    /// Import a private key into an encrypted keystore.
    #[clap(visible_alias = "i")]
    Import {
        /// The name for the account in the keystore.
        #[clap(value_name = "ACCOUNT_NAME")]
        account_name: String,
        /// If provided, keystore will be saved here instead of the default keystores directory
        /// (~/.foundry/keystores)
        #[clap(long, short)]
        keystore_dir: Option<String>,
        #[clap(flatten)]
        raw_wallet_options: RawWallet,
    },
    /// List all the accounts in the keystore default directory
    #[clap(visible_alias = "ls")]
    List,
}

impl WalletSubcommands {
    pub async fn run(self) -> Result<()> {
        match self {
            WalletSubcommands::New { path, unsafe_password, number, json, .. } => {
                let mut rng = thread_rng();

                let mut json_values = if json { Some(vec![] )} else { None };
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


                    for _ in 0..number {
                        let (wallet, uuid) =
                            LocalWallet::new_keystore(&path, &mut rng, password.clone(), None)?;

                        if json {
                            json_values.as_mut().unwrap().push(json!({
                                "address": wallet.address().to_alloy().to_checksum(None),
                                "path": format!("{}", path.join(uuid).display()),
                            }));
                        } else {
                            println!("Created new encrypted keystore file: {}", path.join(uuid).display());
                            println!("Address: {}", wallet.address().to_alloy().to_checksum(None));
                        }
                    }

                    if json {
                        println!("{}", serde_json::to_string_pretty(&json_values.unwrap())?);
                    }
                } else {
                    let mut json_values = if json { Some(vec![] )} else { None };
                    for _ in 0..number {
                        let wallet = LocalWallet::new(&mut rng);

                        if json {
                            json_values.as_mut().unwrap().push(json!({
                                "address": wallet.address().to_alloy().to_checksum(None),
                                "private_key": format!("0x{}", hex::encode(wallet.signer().to_bytes())),
                            }));
                        } else {
                            println!("Successfully created new keypair.");
                            println!("Address:     {}", wallet.address().to_alloy().to_checksum(None));
                            println!("Private key: 0x{}", hex::encode(wallet.signer().to_bytes()));
                        }
                    }

                    if json {
                        println!("{}", serde_json::to_string_pretty(&json_values.unwrap())?);
                    }
                }
            }
            WalletSubcommands::Vanity(cmd) => {
                cmd.run()?;
            }
            WalletSubcommands::Address { wallet, private_key_override } => {
                let wallet = private_key_override
                    .map(|pk| Wallet {
                        raw: RawWallet { private_key: Some(pk), ..Default::default() },
                        ..Default::default()
                    })
                    .unwrap_or(wallet)
                    .signer(0)
                    .await?;
                let addr = wallet.address();
                println!("{}", addr.to_alloy().to_checksum(None));
            }
            WalletSubcommands::Sign { message, data, from_file, wallet } => {
                let wallet = wallet.signer(0).await?;
                let sig = if data {
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
            WalletSubcommands::Verify { message, signature, address } => {
                match signature.verify(Self::hex_str_to_bytes(&message)?, address.to_ethers()) {
                    Ok(_) => {
                        println!("Validation succeeded. Address {address} signed this message.")
                    }
                    Err(_) => {
                        println!("Validation failed. Address {address} did not sign this message.")
                    }
                }
            }
            WalletSubcommands::Import { account_name, keystore_dir, raw_wallet_options } => {
                // Set up keystore directory
                let dir = if let Some(path) = keystore_dir {
                    Path::new(&path).to_path_buf()
                } else {
                    Config::foundry_keystores_dir().ok_or_else(|| {
                        eyre::eyre!("Could not find the default keystore directory.")
                    })?
                };

                fs::create_dir_all(&dir)?;

                // check if account exists already
                let keystore_path = Path::new(&dir).join(&account_name);
                if keystore_path.exists() {
                    eyre::bail!("Keystore file already exists at {}", keystore_path.display());
                }

                // get wallet
                let wallet: Wallet = raw_wallet_options.into();
                let wallet = wallet.try_resolve_local_wallet()?.ok_or_else(|| {
                    eyre::eyre!(
                        "\
Did you set a private key or mnemonic?
Run `cast wallet import --help` and use the corresponding CLI
flag to set your key via:
--private-key, --mnemonic-path or --interactive."
                    )
                })?;

                let private_key = wallet.signer().to_bytes();
                let password = rpassword::prompt_password("Enter password: ")?;

                let mut rng = thread_rng();
                eth_keystore::encrypt_key(
                    &dir,
                    &mut rng,
                    private_key,
                    &password,
                    Some(&account_name),
                )?;
                let address = wallet.address();
                let success_message = format!(
                    "`{}` keystore was saved successfully. Address: {:?}",
                    &account_name, address,
                );
                println!("{}", Paint::green(success_message));
            }
            WalletSubcommands::List => {
                let default_keystore_dir = Config::foundry_keystores_dir()
                    .ok_or_else(|| eyre::eyre!("Could not find the default keystore directory."))?;
                // Create the keystore directory if it doesn't exist
                fs::create_dir_all(&default_keystore_dir)?;
                // List all files in keystore directory
                let keystore_files: Result<Vec<_>, eyre::Report> =
                    std::fs::read_dir(&default_keystore_dir)
                        .wrap_err("Failed to read the directory")?
                        .filter_map(|entry| match entry {
                            Ok(entry) => {
                                let path = entry.path();
                                if path.is_file() && path.extension().is_none() {
                                    Some(Ok(path))
                                } else {
                                    None
                                }
                            }
                            Err(e) => Some(Err(e.into())),
                        })
                        .collect::<Result<Vec<_>, eyre::Report>>();
                // Print the names of the keystore files
                match keystore_files {
                    Ok(files) => {
                        // Print the names of the keystore files
                        for file in files {
                            if let Some(file_name) = file.file_name() {
                                if let Some(name) = file_name.to_str() {
                                    println!("{}", name);
                                }
                            }
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        };

        Ok(())
    }

    fn hex_str_to_bytes(s: &str) -> Result<Vec<u8>> {
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
            WalletSubcommands::Sign { message, data, from_file, .. } => {
                assert_eq!(message, "deadbeef".to_string());
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
            WalletSubcommands::Sign { message, data, from_file, .. } => {
                assert_eq!(message, "0xdeadbeef".to_string());
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
            WalletSubcommands::Sign { message, data, from_file, .. } => {
                assert_eq!(message, "{ ... }".to_string());
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
            WalletSubcommands::Sign { message, data, from_file, .. } => {
                assert_eq!(message, "tests/data/typed_data.json".to_string());
                assert!(data);
                assert!(from_file);
            }
            _ => panic!("expected WalletSubcommands::Sign"),
        }
    }
}
