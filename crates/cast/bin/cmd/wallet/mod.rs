use alloy_primitives::{Address, Signature};
use alloy_signer::{
    coins_bip39::{English, Mnemonic},
    LocalWallet, MnemonicBuilder, Signer as AlloySigner,
};
use clap::Parser;
use ethers_core::types::transaction::eip712::TypedData;
use ethers_signers::Signer;
use eyre::{Context, Result};
use foundry_common::{fs, types::ToAlloy};
use foundry_config::Config;
use foundry_wallets::{RawWalletOpts, WalletOpts, WalletSigner};
use rand::thread_rng;
use serde_json::json;
use std::path::Path;
use yansi::Paint;

pub mod vanity;
use vanity::VanityArgs;

pub mod list;
use list::ListArgs;

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

        /// Number of wallets to generate.
        #[clap(long, short, default_value = "1")]
        number: u32,

        /// Output generated wallets as JSON.
        #[clap(long, short, default_value = "false")]
        json: bool,
    },

    /// Generates a random BIP39 mnemonic phrase
    #[clap(visible_alias = "nm")]
    NewMnemonic {
        /// Number of words for the mnemonic
        #[clap(long, short, default_value = "12")]
        words: usize,

        /// Number of accounts to display
        #[clap(long, short, default_value = "1")]
        accounts: u8,
    },

    /// Generate a vanity address.
    #[clap(visible_alias = "va")]
    Vanity(VanityArgs),

    /// Convert a private key to an address.
    #[clap(visible_aliases = &["a", "addr"])]
    Address {
        /// If provided, the address will be derived from the specified private key.
        #[clap(value_name = "PRIVATE_KEY")]
        private_key_override: Option<String>,

        #[clap(flatten)]
        wallet: WalletOpts,
    },

    /// Sign a message or typed data.
    #[clap(visible_alias = "s")]
    Sign {
        /// The message, typed data, or hash to sign.
        ///
        /// Messages starting with 0x are expected to be hex encoded, which get decoded before
        /// being signed.
        ///
        /// The message will be prefixed with the Ethereum Signed Message header and hashed before
        /// signing, unless `--no-hash` is provided.
        ///
        /// Typed data can be provided as a json string or a file name.
        /// Use --data flag to denote the message is a string of typed data.
        /// Use --data --from-file to denote the message is a file name containing typed data.
        /// The data will be combined and hashed using the EIP712 specification before signing.
        /// The data should be formatted as JSON.
        message: String,

        /// Treat the message as JSON typed data.
        #[clap(long)]
        data: bool,

        /// Treat the message as a file containing JSON typed data. Requires `--data`.
        #[clap(long, requires = "data")]
        from_file: bool,

        /// Treat the message as a raw 32-byte hash and sign it directly without hashing it again.
        #[clap(long, conflicts_with = "data")]
        no_hash: bool,

        #[clap(flatten)]
        wallet: WalletOpts,
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
        raw_wallet_options: RawWalletOpts,
    },
    /// List all the accounts in the keystore default directory
    #[clap(visible_alias = "ls")]
    List(ListArgs),

    /// Derives private key from mnemonic
    #[clap(name = "derive-private-key", visible_aliases = &["--derive-private-key"])]
    DerivePrivateKey { mnemonic: String, mnemonic_index: Option<u8> },
}

impl WalletSubcommands {
    pub async fn run(self) -> Result<()> {
        match self {
            WalletSubcommands::New { path, unsafe_password, number, json, .. } => {
                let mut rng = thread_rng();

                let mut json_values = if json { Some(vec![]) } else { None };
                if let Some(path) = path {
                    let path = match dunce::canonicalize(path.clone()) {
                        Ok(path) => path,
                        // If the path doesn't exist, it will fail to be canonicalized,
                        // so we attach more context to the error message.
                        Err(e) => {
                            eyre::bail!("If you specified a directory, please make sure it exists, or create it before running `cast wallet new <DIR>`.\n{path} is not a directory.\nError: {}", e);
                        }
                    };
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

                        if let Some(json) = json_values.as_mut() {
                            json.push(json!({
                                "address": wallet.address().to_checksum(None),
                                "path": format!("{}", path.join(uuid).display()),
                            }
                            ));
                        } else {
                            println!(
                                "Created new encrypted keystore file: {}",
                                path.join(uuid).display()
                            );
                            println!("Address: {}", wallet.address().to_checksum(None));
                        }
                    }

                    if let Some(json) = json_values.as_ref() {
                        println!("{}", serde_json::to_string_pretty(json)?);
                    }
                } else {
                    for _ in 0..number {
                        let wallet = LocalWallet::random_with(&mut rng);

                        if let Some(json) = json_values.as_mut() {
                            json.push(json!({
                                "address": wallet.address().to_checksum(None),
                                "private_key": format!("0x{}", hex::encode(wallet.signer().to_bytes())),
                            }))
                        } else {
                            println!("Successfully created new keypair.");
                            println!("Address:     {}", wallet.address().to_checksum(None));
                            println!("Private key: 0x{}", hex::encode(wallet.signer().to_bytes()));
                        }
                    }

                    if let Some(json) = json_values.as_ref() {
                        println!("{}", serde_json::to_string_pretty(json)?);
                    }
                }
            }
            WalletSubcommands::NewMnemonic { words, accounts } => {
                let mut rng = thread_rng();
                let phrase = Mnemonic::<English>::new_with_count(&mut rng, words)?.to_phrase();

                let builder = MnemonicBuilder::<English>::default().phrase(phrase.as_str());
                let derivation_path = "m/44'/60'/0'/0/";
                let wallets = (0..accounts)
                    .map(|i| builder.clone().derivation_path(format!("{derivation_path}{i}")))
                    .collect::<Result<Vec<_>, _>>()?;
                let wallets =
                    wallets.into_iter().map(|b| b.build()).collect::<Result<Vec<_>, _>>()?;

                println!("{}", Paint::green("Successfully generated a new mnemonic."));
                println!("Phrase:\n{phrase}");
                println!("\nAccounts:");
                for (i, wallet) in wallets.iter().enumerate() {
                    println!("- Account {i}:");
                    println!("Address:     {}", wallet.address());
                    println!("Private key: 0x{}\n", hex::encode(wallet.signer().to_bytes()));
                }
            }
            WalletSubcommands::Vanity(cmd) => {
                cmd.run()?;
            }
            WalletSubcommands::Address { wallet, private_key_override } => {
                let wallet = private_key_override
                    .map(|pk| WalletOpts {
                        raw: RawWalletOpts { private_key: Some(pk), ..Default::default() },
                        ..Default::default()
                    })
                    .unwrap_or(wallet)
                    .signer()
                    .await?;
                let addr = wallet.address();
                println!("{}", addr.to_alloy().to_checksum(None));
            }
            WalletSubcommands::Sign { message, data, from_file, no_hash, wallet } => {
                let wallet = wallet.signer().await?;
                let sig = if data {
                    let typed_data: TypedData = if from_file {
                        // data is a file name, read json from file
                        foundry_common::fs::read_json_file(message.as_ref())?
                    } else {
                        // data is a json string
                        serde_json::from_str(&message)?
                    };
                    wallet.sign_typed_data(&typed_data).await?
                } else if no_hash {
                    wallet.sign_hash(&message.parse()?).await?
                } else {
                    wallet.sign_message(Self::hex_str_to_bytes(&message)?).await?
                };
                println!("0x{sig}");
            }
            WalletSubcommands::Verify { message, signature, address } => {
                let recovered_address = Self::recover_address_from_message(&message, &signature)?;
                if address == recovered_address {
                    println!("Validation succeeded. Address {address} signed this message.");
                } else {
                    println!("Validation failed. Address {address} did not sign this message.");
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
                let wallet = raw_wallet_options
                    .signer()?
                    .and_then(|s| match s {
                        WalletSigner::Local(s) => Some(s),
                        _ => None,
                    })
                    .ok_or_else(|| {
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
            WalletSubcommands::List(cmd) => {
                cmd.run().await?;
            }
            WalletSubcommands::DerivePrivateKey { mnemonic, mnemonic_index } => {
                let phrase = Mnemonic::<English>::new_from_phrase(mnemonic.as_str())?.to_phrase();
                let builder = MnemonicBuilder::<English>::default().phrase(phrase.as_str());
                let derivation_path = "m/44'/60'/0'/0/";
                let index = if let Some(i) = mnemonic_index { i } else { 0 };
                let wallet = builder
                    .clone()
                    .derivation_path(format!("{derivation_path}{index}"))?
                    .build()?;
                println!("- Account:");
                println!("Address:     {}", wallet.address());
                println!("Private key: 0x{}\n", hex::encode(wallet.signer().to_bytes()));
            }
        };

        Ok(())
    }

    /// Recovers an address from the specified message and signature
    fn recover_address_from_message(message: &str, signature: &Signature) -> Result<Address> {
        Ok(signature.recover_address_from_msg(message)?)
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
    use std::str::FromStr;

    use alloy_primitives::address;

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
    fn can_verify_signed_hex_message() {
        let message = "hello";
        let signature = Signature::from_str("f2dd00eac33840c04b6fc8a5ec8c4a47eff63575c2bc7312ecb269383de0c668045309c423484c8d097df306e690c653f8e1ec92f7f6f45d1f517027771c3e801c").unwrap();
        let address = address!("28A4F420a619974a2393365BCe5a7b560078Cc13");
        let recovered_address =
            WalletSubcommands::recover_address_from_message(message, &signature);
        assert!(recovered_address.is_ok());
        assert_eq!(address, recovered_address.unwrap());
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
