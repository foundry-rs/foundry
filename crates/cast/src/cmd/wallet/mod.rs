use alloy_chains::Chain;
use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, B256, Signature, U256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::Authorization;
use alloy_signer::Signer;
use alloy_signer_local::{
    MnemonicBuilder, PrivateKeySigner,
    coins_bip39::{English, Entropy, Mnemonic},
};
use clap::Parser;
use eyre::{Context, Result};
use foundry_cli::{opts::RpcOpts, utils, utils::LoadConfig};
use foundry_common::{fs, sh_println, shell};
use foundry_config::Config;
use foundry_wallets::{RawWalletOpts, WalletOpts, WalletSigner};
use rand_08::thread_rng;
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
    #[command(visible_alias = "n")]
    New {
        /// If provided, then keypair will be written to an encrypted JSON keystore.
        path: Option<String>,

        /// Account name for the keystore file. If provided, the keystore file
        /// will be named using this account name.
        #[arg(value_name = "ACCOUNT_NAME")]
        account_name: Option<String>,

        /// Triggers a hidden password prompt for the JSON keystore.
        ///
        /// Deprecated: prompting for a hidden password is now the default.
        #[arg(long, short, conflicts_with = "unsafe_password")]
        password: bool,

        /// Password for the JSON keystore in cleartext.
        ///
        /// This is UNSAFE to use and we recommend using the --password.
        #[arg(long, env = "CAST_PASSWORD", value_name = "PASSWORD")]
        unsafe_password: Option<String>,

        /// Number of wallets to generate.
        #[arg(long, short, default_value = "1")]
        number: u32,
    },

    /// Generates a random BIP39 mnemonic phrase
    #[command(visible_alias = "nm")]
    NewMnemonic {
        /// Number of words for the mnemonic
        #[arg(long, short, default_value = "12")]
        words: usize,

        /// Number of accounts to display
        #[arg(long, short, default_value = "1")]
        accounts: u8,

        /// Entropy to use for the mnemonic
        #[arg(long, short, conflicts_with = "words")]
        entropy: Option<String>,
    },

    /// Generate a vanity address.
    #[command(visible_alias = "va")]
    Vanity(VanityArgs),

    /// Convert a private key to an address.
    #[command(visible_aliases = &["a", "addr"])]
    Address {
        /// If provided, the address will be derived from the specified private key.
        #[arg(value_name = "PRIVATE_KEY")]
        private_key_override: Option<String>,

        #[command(flatten)]
        wallet: WalletOpts,
    },

    /// Derive accounts from a mnemonic
    #[command(visible_alias = "d")]
    Derive {
        /// The accounts will be derived from the specified mnemonic phrase.
        #[arg(value_name = "MNEMONIC")]
        mnemonic: String,

        /// Number of accounts to display.
        #[arg(long, short, default_value = "1")]
        accounts: Option<u8>,

        /// Insecure mode: display private keys in the terminal.
        #[arg(long, default_value = "false")]
        insecure: bool,
    },

    /// Sign a message or typed data.
    #[command(visible_alias = "s")]
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
        #[arg(long)]
        data: bool,

        /// Treat the message as a file containing JSON typed data. Requires `--data`.
        #[arg(long, requires = "data")]
        from_file: bool,

        /// Treat the message as a raw 32-byte hash and sign it directly without hashing it again.
        #[arg(long, conflicts_with = "data")]
        no_hash: bool,

        #[command(flatten)]
        wallet: WalletOpts,
    },

    /// EIP-7702 sign authorization.
    #[command(visible_alias = "sa")]
    SignAuth {
        /// Address to sign authorization for.
        address: Address,

        #[command(flatten)]
        rpc: RpcOpts,

        #[arg(long)]
        nonce: Option<u64>,

        #[arg(long)]
        chain: Option<Chain>,

        /// If set, indicates the authorization will be broadcast by the signing account itself.
        /// This means the nonce used will be the current nonce + 1 (to account for the
        /// transaction that will include this authorization).
        #[arg(long, conflicts_with = "nonce")]
        self_broadcast: bool,

        #[command(flatten)]
        wallet: WalletOpts,
    },

    /// Verify the signature of a message.
    #[command(visible_alias = "v")]
    Verify {
        /// The original message.
        ///
        /// Treats 0x-prefixed strings as hex encoded bytes.
        /// Non 0x-prefixed strings are treated as raw input message.
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

        /// The signature to verify.
        signature: Signature,

        /// The address of the message signer.
        #[arg(long, short)]
        address: Address,

        /// Treat the message as JSON typed data.
        #[arg(long)]
        data: bool,

        /// Treat the message as a file containing JSON typed data. Requires `--data`.
        #[arg(long, requires = "data")]
        from_file: bool,

        /// Treat the message as a raw 32-byte hash and sign it directly without hashing it again.
        #[arg(long, conflicts_with = "data")]
        no_hash: bool,
    },

    /// Import a private key into an encrypted keystore.
    #[command(visible_alias = "i")]
    Import {
        /// The name for the account in the keystore.
        #[arg(value_name = "ACCOUNT_NAME")]
        account_name: String,
        /// If provided, keystore will be saved here instead of the default keystores directory
        /// (~/.foundry/keystores)
        #[arg(long, short)]
        keystore_dir: Option<String>,
        /// Password for the JSON keystore in cleartext
        /// This is unsafe, we recommend using the default hidden password prompt
        #[arg(long, env = "CAST_UNSAFE_PASSWORD", value_name = "PASSWORD")]
        unsafe_password: Option<String>,
        #[command(flatten)]
        raw_wallet_options: RawWalletOpts,
    },

    /// List all the accounts in the keystore default directory
    #[command(visible_alias = "ls")]
    List(ListArgs),

    /// Remove a wallet from the keystore.
    ///
    /// This command requires the wallet alias and will prompt for a password to ensure that only
    /// an authorized user can remove the wallet.
    #[command(visible_aliases = &["rm"], override_usage = "cast wallet remove --name <NAME>")]
    Remove {
        /// The alias (or name) of the wallet to remove.
        #[arg(long, required = true)]
        name: String,
        /// Optionally provide the keystore directory if not provided. default directory will be
        /// used (~/.foundry/keystores).
        #[arg(long)]
        dir: Option<String>,
        /// Password for the JSON keystore in cleartext
        /// This is unsafe, we recommend using the default hidden password prompt
        #[arg(long, env = "CAST_UNSAFE_PASSWORD", value_name = "PASSWORD")]
        unsafe_password: Option<String>,
    },

    /// Derives private key from mnemonic
    #[command(name = "private-key", visible_alias = "pk", aliases = &["derive-private-key", "--derive-private-key"])]
    PrivateKey {
        /// If provided, the private key will be derived from the specified mnemonic phrase.
        #[arg(value_name = "MNEMONIC")]
        mnemonic_override: Option<String>,

        /// If provided, the private key will be derived using the
        /// specified mnemonic index (if integer) or derivation path.
        #[arg(value_name = "MNEMONIC_INDEX_OR_DERIVATION_PATH")]
        mnemonic_index_or_derivation_path_override: Option<String>,

        #[command(flatten)]
        wallet: WalletOpts,
    },
    /// Get the public key for the given private key.
    #[command(visible_aliases = &["pubkey"])]
    PublicKey {
        /// If provided, the public key will be derived from the specified private key.
        #[arg(long = "raw-private-key", value_name = "PRIVATE_KEY")]
        private_key_override: Option<String>,

        #[command(flatten)]
        wallet: WalletOpts,
    },
    /// Decrypt a keystore file to get the private key
    #[command(name = "decrypt-keystore", visible_alias = "dk")]
    DecryptKeystore {
        /// The name for the account in the keystore.
        #[arg(value_name = "ACCOUNT_NAME")]
        account_name: String,
        /// If not provided, keystore will try to be located at the default keystores directory
        /// (~/.foundry/keystores)
        #[arg(long, short)]
        keystore_dir: Option<String>,
        /// Password for the JSON keystore in cleartext
        /// This is unsafe, we recommend using the default hidden password prompt
        #[arg(long, env = "CAST_UNSAFE_PASSWORD", value_name = "PASSWORD")]
        unsafe_password: Option<String>,
    },

    /// Change the password of a keystore file
    #[command(name = "change-password", visible_alias = "cp")]
    ChangePassword {
        /// The name for the account in the keystore.
        #[arg(value_name = "ACCOUNT_NAME")]
        account_name: String,
        /// If not provided, keystore will try to be located at the default keystores directory
        /// (~/.foundry/keystores)
        #[arg(long, short)]
        keystore_dir: Option<String>,
        /// Current password for the JSON keystore in cleartext
        /// This is unsafe, we recommend using the default hidden password prompt
        #[arg(long, env = "CAST_UNSAFE_PASSWORD", value_name = "PASSWORD")]
        unsafe_password: Option<String>,
        /// New password for the JSON keystore in cleartext
        /// This is unsafe, we recommend using the default hidden password prompt
        #[arg(long, env = "CAST_UNSAFE_NEW_PASSWORD", value_name = "NEW_PASSWORD")]
        unsafe_new_password: Option<String>,
    },
}

impl WalletSubcommands {
    pub async fn run(self) -> Result<()> {
        match self {
            Self::New { path, account_name, unsafe_password, number, password } => {
                let mut rng = thread_rng();

                let mut json_values = if shell::is_json() { Some(vec![]) } else { None };

                let path = if let Some(path) = path {
                    match dunce::canonicalize(&path) {
                        Ok(path) => {
                            if !path.is_dir() {
                                // we require path to be an existing directory
                                eyre::bail!("`{}` is not a directory", path.display());
                            }
                            Some(path)
                        }
                        Err(e) => {
                            eyre::bail!(
                                "If you specified a directory, please make sure it exists, or create it before running `cast wallet new <DIR>`.\n{path} is not a directory.\nError: {}",
                                e
                            );
                        }
                    }
                } else if unsafe_password.is_some() || password {
                    let path = Config::foundry_keystores_dir().ok_or_else(|| {
                        eyre::eyre!("Could not find the default keystore directory.")
                    })?;
                    fs::create_dir_all(&path)?;
                    Some(path)
                } else {
                    None
                };

                match path {
                    Some(path) => {
                        let password = if let Some(password) = unsafe_password {
                            password
                        } else {
                            // if no --unsafe-password was provided read via stdin
                            rpassword::prompt_password("Enter secret: ")?
                        };

                        for i in 0..number {
                            let account_name_ref =
                                account_name.as_deref().map(|name| match number {
                                    1 => name.to_string(),
                                    _ => format!("{}_{}", name, i + 1),
                                });

                            let (wallet, uuid) = PrivateKeySigner::new_keystore(
                                &path,
                                &mut rng,
                                password.clone(),
                                account_name_ref.as_deref(),
                            )?;
                            let identifier = account_name_ref.as_deref().unwrap_or(&uuid);

                            if let Some(json) = json_values.as_mut() {
                                json.push(if shell::verbosity() > 0 {
                                json!({
                                    "address": wallet.address().to_checksum(None),
                                    "public_key": format!("0x{}", hex::encode(wallet.public_key())),
                                    "path": format!("{}", path.join(identifier).display()),
                                })
                            } else {
                                json!({
                                    "address": wallet.address().to_checksum(None),
                                    "path": format!("{}", path.join(identifier).display()),
                                })
                            });
                            } else {
                                sh_println!(
                                    "Created new encrypted keystore file: {}",
                                    path.join(identifier).display()
                                )?;
                                sh_println!("Address:    {}", wallet.address().to_checksum(None))?;
                                if shell::verbosity() > 0 {
                                    sh_println!(
                                        "Public key: 0x{}",
                                        hex::encode(wallet.public_key())
                                    )?;
                                }
                            }
                        }
                    }
                    None => {
                        for _ in 0..number {
                            let wallet = PrivateKeySigner::random_with(&mut rng);

                            if let Some(json) = json_values.as_mut() {
                                json.push(if shell::verbosity() > 0 {
                                json!({
                                    "address": wallet.address().to_checksum(None),
                                    "public_key": format!("0x{}", hex::encode(wallet.public_key())),
                                    "private_key": format!("0x{}", hex::encode(wallet.credential().to_bytes())),
                                })
                            } else {
                                json!({
                                    "address": wallet.address().to_checksum(None),
                                    "private_key": format!("0x{}", hex::encode(wallet.credential().to_bytes())),
                                })
                            });
                            } else {
                                sh_println!("Successfully created new keypair.")?;
                                sh_println!("Address:     {}", wallet.address().to_checksum(None))?;
                                if shell::verbosity() > 0 {
                                    sh_println!(
                                        "Public key:  0x{}",
                                        hex::encode(wallet.public_key())
                                    )?;
                                }
                                sh_println!(
                                    "Private key: 0x{}",
                                    hex::encode(wallet.credential().to_bytes())
                                )?;
                            }
                        }
                    }
                }

                if let Some(json) = json_values.as_ref() {
                    sh_println!("{}", serde_json::to_string_pretty(json)?)?;
                }
            }
            Self::NewMnemonic { words, accounts, entropy } => {
                let phrase = if let Some(entropy) = entropy {
                    let entropy = Entropy::from_slice(hex::decode(entropy)?)?;
                    Mnemonic::<English>::new_from_entropy(entropy).to_phrase()
                } else {
                    let mut rng = thread_rng();
                    Mnemonic::<English>::new_with_count(&mut rng, words)?.to_phrase()
                };

                let format_json = shell::is_json();

                if !format_json {
                    sh_println!("{}", "Generating mnemonic from provided entropy...".yellow())?;
                }

                let builder = MnemonicBuilder::<English>::default().phrase(phrase.as_str());
                let derivation_path = "m/44'/60'/0'/0/";
                let wallets = (0..accounts)
                    .map(|i| builder.clone().derivation_path(format!("{derivation_path}{i}")))
                    .collect::<Result<Vec<_>, _>>()?;
                let wallets =
                    wallets.into_iter().map(|b| b.build()).collect::<Result<Vec<_>, _>>()?;

                if !format_json {
                    sh_println!("{}", "Successfully generated a new mnemonic.".green())?;
                    sh_println!("Phrase:\n{phrase}")?;
                    sh_println!("\nAccounts:")?;
                }

                let mut accounts = json!([]);
                for (i, wallet) in wallets.iter().enumerate() {
                    let public_key = hex::encode(wallet.public_key());
                    let private_key = hex::encode(wallet.credential().to_bytes());
                    if format_json {
                        accounts.as_array_mut().unwrap().push(if shell::verbosity() > 0 {
                            json!({
                                "address": format!("{}", wallet.address()),
                                "public_key": format!("0x{}", public_key),
                                "private_key": format!("0x{}", private_key),
                            })
                        } else {
                            json!({
                                "address": format!("{}", wallet.address()),
                                "private_key": format!("0x{}", private_key),
                            })
                        });
                    } else {
                        sh_println!("- Account {i}:")?;
                        sh_println!("Address:     {}", wallet.address())?;
                        if shell::verbosity() > 0 {
                            sh_println!("Public key:  0x{}", public_key)?;
                        }
                        sh_println!("Private key: 0x{}\n", private_key)?;
                    }
                }

                if format_json {
                    let obj = json!({
                        "mnemonic": phrase,
                        "accounts": accounts,
                    });
                    sh_println!("{}", serde_json::to_string_pretty(&obj)?)?;
                }
            }
            Self::Vanity(cmd) => {
                cmd.run()?;
            }
            Self::Address { wallet, private_key_override } => {
                let wallet = private_key_override
                    .map(|pk| WalletOpts {
                        raw: RawWalletOpts { private_key: Some(pk), ..Default::default() },
                        ..Default::default()
                    })
                    .unwrap_or(wallet)
                    .signer()
                    .await?;
                let addr = wallet.address();
                sh_println!("{}", addr.to_checksum(None))?;
            }
            Self::Derive { mnemonic, accounts, insecure } => {
                let format_json = shell::is_json();
                let mut accounts_json = json!([]);
                for i in 0..accounts.unwrap_or(1) {
                    let wallet = WalletOpts {
                        raw: RawWalletOpts {
                            mnemonic: Some(mnemonic.clone()),
                            mnemonic_index: i as u32,
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                    .signer()
                    .await?;

                    match wallet {
                        WalletSigner::Local(local_wallet) => {
                            let address = local_wallet.address().to_checksum(None);
                            let private_key = hex::encode(local_wallet.credential().to_bytes());
                            if format_json {
                                if insecure {
                                    accounts_json.as_array_mut().unwrap().push(json!({
                                        "address": format!("{}", address),
                                        "private_key": format!("0x{}", private_key),
                                    }));
                                } else {
                                    accounts_json.as_array_mut().unwrap().push(json!({
                                        "address": format!("{}", address)
                                    }));
                                }
                            } else {
                                sh_println!("- Account {i}:")?;
                                if insecure {
                                    sh_println!("Address:     {}", address)?;
                                    sh_println!("Private key: 0x{}\n", private_key)?;
                                } else {
                                    sh_println!("Address:     {}\n", address)?;
                                }
                            }
                        }
                        _ => eyre::bail!("Only local wallets are supported by this command"),
                    }
                }

                if format_json {
                    sh_println!("{}", serde_json::to_string_pretty(&accounts_json)?)?;
                }
            }
            Self::PublicKey { wallet, private_key_override } => {
                let wallet = private_key_override
                    .map(|pk| WalletOpts {
                        raw: RawWalletOpts { private_key: Some(pk), ..Default::default() },
                        ..Default::default()
                    })
                    .unwrap_or(wallet)
                    .signer()
                    .await?;

                let public_key = match wallet {
                    WalletSigner::Local(wallet) => wallet.public_key(),
                    _ => eyre::bail!("Only local wallets are supported by this command"),
                };

                sh_println!("0x{}", hex::encode(public_key))?;
            }
            Self::Sign { message, data, from_file, no_hash, wallet } => {
                let wallet = wallet.signer().await?;
                let sig = if data {
                    let typed_data: TypedData = if from_file {
                        // data is a file name, read json from file
                        foundry_common::fs::read_json_file(message.as_ref())?
                    } else {
                        // data is a json string
                        serde_json::from_str(&message)?
                    };
                    wallet.sign_dynamic_typed_data(&typed_data).await?
                } else if no_hash {
                    wallet.sign_hash(&hex::decode(&message)?[..].try_into()?).await?
                } else {
                    wallet.sign_message(&Self::hex_str_to_bytes(&message)?).await?
                };

                if shell::verbosity() > 0 {
                    if shell::is_json() {
                        sh_println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "message": message,
                                "address": wallet.address(),
                                "signature": hex::encode(sig.as_bytes()),
                            }))?
                        )?;
                    } else {
                        sh_println!(
                            "Successfully signed!\n   Message: {}\n   Address: {}\n   Signature: 0x{}",
                            message,
                            wallet.address(),
                            hex::encode(sig.as_bytes()),
                        )?;
                    }
                } else {
                    // Pipe friendly output
                    sh_println!("0x{}", hex::encode(sig.as_bytes()))?;
                }
            }
            Self::SignAuth { rpc, nonce, chain, wallet, address, self_broadcast } => {
                let wallet = wallet.signer().await?;
                let provider = utils::get_provider(&rpc.load_config()?)?;
                let nonce = if let Some(nonce) = nonce {
                    nonce
                } else {
                    let current_nonce = provider.get_transaction_count(wallet.address()).await?;
                    if self_broadcast {
                        // When self-broadcasting, the authorization nonce needs to be +1
                        // because the transaction itself will consume the current nonce
                        current_nonce + 1
                    } else {
                        current_nonce
                    }
                };
                let chain_id = if let Some(chain) = chain {
                    chain.id()
                } else {
                    provider.get_chain_id().await?
                };
                let auth = Authorization { chain_id: U256::from(chain_id), address, nonce };
                let signature = wallet.sign_hash(&auth.signature_hash()).await?;
                let auth = auth.into_signed(signature);

                if shell::verbosity() > 0 {
                    if shell::is_json() {
                        sh_println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "nonce": nonce,
                                "chain_id": chain_id,
                                "address": wallet.address(),
                                "signature": hex::encode_prefixed(alloy_rlp::encode(&auth)),
                            }))?
                        )?;
                    } else {
                        sh_println!(
                            "Successfully signed!\n   Nonce: {}\n   Chain ID: {}\n   Address: {}\n   Signature: 0x{}",
                            nonce,
                            chain_id,
                            wallet.address(),
                            hex::encode_prefixed(alloy_rlp::encode(&auth)),
                        )?;
                    }
                } else {
                    // Pipe friendly output
                    sh_println!("{}", hex::encode_prefixed(alloy_rlp::encode(&auth)))?;
                }
            }
            Self::Verify { message, signature, address, data, from_file, no_hash } => {
                let recovered_address = if data {
                    let typed_data: TypedData = if from_file {
                        // data is a file name, read json from file
                        foundry_common::fs::read_json_file(message.as_ref())?
                    } else {
                        // data is a json string
                        serde_json::from_str(&message)?
                    };
                    Self::recover_address_from_typed_data(&typed_data, &signature)?
                } else if no_hash {
                    Self::recover_address_from_message_no_hash(
                        &hex::decode(&message)?[..].try_into()?,
                        &signature,
                    )?
                } else {
                    Self::recover_address_from_message(&message, &signature)?
                };

                if address == recovered_address {
                    sh_println!("Validation succeeded. Address {address} signed this message.")?;
                } else {
                    eyre::bail!("Validation failed. Address {address} did not sign this message.");
                }
            }
            Self::Import { account_name, keystore_dir, unsafe_password, raw_wallet_options } => {
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

                let private_key = wallet.credential().to_bytes();
                let password = if let Some(password) = unsafe_password {
                    password
                } else {
                    // if no --unsafe-password was provided read via stdin
                    rpassword::prompt_password("Enter password: ")?
                };

                let mut rng = thread_rng();
                let (wallet, _) = PrivateKeySigner::encrypt_keystore(
                    dir,
                    &mut rng,
                    private_key,
                    password,
                    Some(&account_name),
                )?;
                let address = wallet.address();
                let success_message = format!(
                    "`{}` keystore was saved successfully. Address: {:?}",
                    &account_name, address,
                );
                sh_println!("{}", success_message.green())?;
            }
            Self::List(cmd) => {
                cmd.run().await?;
            }
            Self::Remove { name, dir, unsafe_password } => {
                let dir = if let Some(path) = dir {
                    Path::new(&path).to_path_buf()
                } else {
                    Config::foundry_keystores_dir().ok_or_else(|| {
                        eyre::eyre!("Could not find the default keystore directory.")
                    })?
                };

                let keystore_path = Path::new(&dir).join(&name);
                if !keystore_path.exists() {
                    eyre::bail!("Keystore file does not exist at {}", keystore_path.display());
                }

                let password = if let Some(pwd) = unsafe_password {
                    pwd
                } else {
                    rpassword::prompt_password("Enter password: ")?
                };

                if PrivateKeySigner::decrypt_keystore(&keystore_path, password).is_err() {
                    eyre::bail!("Invalid password - wallet removal cancelled");
                }

                std::fs::remove_file(&keystore_path).wrap_err_with(|| {
                    format!("Failed to remove keystore file at {}", keystore_path.display())
                })?;

                let success_message = format!("`{}` keystore was removed successfully.", &name);
                sh_println!("{}", success_message.green())?;
            }
            Self::PrivateKey {
                wallet,
                mnemonic_override,
                mnemonic_index_or_derivation_path_override,
            } => {
                let (index_override, derivation_path_override) =
                    match mnemonic_index_or_derivation_path_override {
                        Some(value) => match value.parse::<u32>() {
                            Ok(index) => (Some(index), None),
                            Err(_) => (None, Some(value)),
                        },
                        None => (None, None),
                    };
                let wallet = WalletOpts {
                    raw: RawWalletOpts {
                        mnemonic: mnemonic_override.or(wallet.raw.mnemonic),
                        mnemonic_index: index_override.unwrap_or(wallet.raw.mnemonic_index),
                        hd_path: derivation_path_override.or(wallet.raw.hd_path),
                        ..wallet.raw
                    },
                    ..wallet
                }
                .signer()
                .await?;
                match wallet {
                    WalletSigner::Local(wallet) => {
                        if shell::verbosity() > 0 {
                            sh_println!("Address:     {}", wallet.address())?;
                            sh_println!(
                                "Private key: 0x{}",
                                hex::encode(wallet.credential().to_bytes())
                            )?;
                        } else {
                            sh_println!("0x{}", hex::encode(wallet.credential().to_bytes()))?;
                        }
                    }
                    _ => {
                        eyre::bail!("Only local wallets are supported by this command.");
                    }
                }
            }
            Self::DecryptKeystore { account_name, keystore_dir, unsafe_password } => {
                // Set up keystore directory
                let dir = if let Some(path) = keystore_dir {
                    Path::new(&path).to_path_buf()
                } else {
                    Config::foundry_keystores_dir().ok_or_else(|| {
                        eyre::eyre!("Could not find the default keystore directory.")
                    })?
                };

                let keypath = dir.join(&account_name);

                if !keypath.exists() {
                    eyre::bail!("Keystore file does not exist at {}", keypath.display());
                }

                let password = if let Some(password) = unsafe_password {
                    password
                } else {
                    // if no --unsafe-password was provided read via stdin
                    rpassword::prompt_password("Enter password: ")?
                };

                let wallet = PrivateKeySigner::decrypt_keystore(keypath, password)?;

                let private_key = B256::from_slice(&wallet.credential().to_bytes());

                let success_message =
                    format!("{}'s private key is: {}", &account_name, private_key);

                sh_println!("{}", success_message.green())?;
            }
            Self::ChangePassword {
                account_name,
                keystore_dir,
                unsafe_password,
                unsafe_new_password,
            } => {
                // Set up keystore directory
                let dir = if let Some(path) = keystore_dir {
                    Path::new(&path).to_path_buf()
                } else {
                    Config::foundry_keystores_dir().ok_or_else(|| {
                        eyre::eyre!("Could not find the default keystore directory.")
                    })?
                };

                let keypath = dir.join(&account_name);

                if !keypath.exists() {
                    eyre::bail!("Keystore file does not exist at {}", keypath.display());
                }

                let current_password = if let Some(password) = unsafe_password {
                    password
                } else {
                    // if no --unsafe-password was provided read via stdin
                    rpassword::prompt_password("Enter current password: ")?
                };

                // decrypt the keystore to verify the current password and get the private key
                let wallet = PrivateKeySigner::decrypt_keystore(&keypath, current_password.clone())
                    .map_err(|_| eyre::eyre!("Invalid password - password change cancelled"))?;

                let new_password = if let Some(password) = unsafe_new_password {
                    password
                } else {
                    // if no --unsafe-new-password was provided read via stdin
                    rpassword::prompt_password("Enter new password: ")?
                };

                if current_password == new_password {
                    eyre::bail!("New password cannot be the same as the current password");
                }

                // Create a new keystore with the new password
                let private_key = wallet.credential().to_bytes();
                let mut rng = thread_rng();
                let (wallet, _) = PrivateKeySigner::encrypt_keystore(
                    dir,
                    &mut rng,
                    private_key,
                    new_password,
                    Some(&account_name),
                )?;

                let success_message = format!(
                    "Password for keystore `{}` was changed successfully. Address: {:?}",
                    &account_name,
                    wallet.address(),
                );
                sh_println!("{}", success_message.green())?;
            }
        };

        Ok(())
    }

    /// Recovers an address from the specified message and signature.
    ///
    /// Note: This attempts to decode the message as hex if it starts with 0x.
    fn recover_address_from_message(message: &str, signature: &Signature) -> Result<Address> {
        let message = Self::hex_str_to_bytes(message)?;
        Ok(signature.recover_address_from_msg(message)?)
    }

    /// Recovers an address from the specified message and signature.
    fn recover_address_from_message_no_hash(
        prehash: &B256,
        signature: &Signature,
    ) -> Result<Address> {
        Ok(signature.recover_address_from_prehash(prehash)?)
    }

    /// Recovers an address from the specified EIP-712 typed data and signature.
    fn recover_address_from_typed_data(
        typed_data: &TypedData,
        signature: &Signature,
    ) -> Result<Address> {
        Ok(signature.recover_address_from_prehash(&typed_data.eip712_signing_hash()?)?)
    }

    /// Strips the 0x prefix from a hex string and decodes it to bytes.
    ///
    /// Treats the string as raw bytes if it doesn't start with 0x.
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
    use alloy_primitives::{address, keccak256};
    use std::str::FromStr;

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
        let address = address!("0x28A4F420a619974a2393365BCe5a7b560078Cc13");
        let recovered_address =
            WalletSubcommands::recover_address_from_message(message, &signature);
        assert!(recovered_address.is_ok());
        assert_eq!(address, recovered_address.unwrap());
    }

    #[test]
    fn can_verify_signed_hex_message_no_hash() {
        let prehash = keccak256("hello");
        let signature = Signature::from_str("433ec3d37e4f1253df15e2dea412fed8e915737730f74b3dfb1353268f932ef5557c9158e0b34bce39de28d11797b42e9b1acb2749230885fe075aedc3e491a41b").unwrap();
        let address = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"); // private key = 1
        let recovered_address =
            WalletSubcommands::recover_address_from_message_no_hash(&prehash, &signature);
        assert!(recovered_address.is_ok());
        assert_eq!(address, recovered_address.unwrap());
    }

    #[test]
    fn can_verify_signed_typed_data() {
        let typed_data: TypedData = serde_json::from_str(r#"{"domain":{"name":"Test","version":"1","chainId":1,"verifyingContract":"0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF"},"message":{"value":123},"primaryType":"Data","types":{"Data":[{"name":"value","type":"uint256"}]}}"#).unwrap();
        let signature = Signature::from_str("0285ff83b93bd01c14e201943af7454fe2bc6c98be707a73888c397d6ae3b0b92f73ca559f81cbb19fe4e0f1dc4105bd7b647c6a84b033057977cf2ec982daf71b").unwrap();
        let address = address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"); // private key = 1
        let recovered_address =
            WalletSubcommands::recover_address_from_typed_data(&typed_data, &signature);
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

    #[test]
    fn can_parse_wallet_change_password() {
        let args = WalletSubcommands::parse_from([
            "foundry-cli",
            "change-password",
            "my_account",
            "--unsafe-password",
            "old_password",
            "--unsafe-new-password",
            "new_password",
        ]);
        match args {
            WalletSubcommands::ChangePassword {
                account_name,
                keystore_dir,
                unsafe_password,
                unsafe_new_password,
            } => {
                assert_eq!(account_name, "my_account".to_string());
                assert_eq!(unsafe_password, Some("old_password".to_string()));
                assert_eq!(unsafe_new_password, Some("new_password".to_string()));
                assert!(keystore_dir.is_none());
            }
            _ => panic!("expected WalletSubcommands::ChangePassword"),
        }
    }

    #[test]
    fn wallet_sign_auth_nonce_and_self_broadcast_conflict() {
        let result = WalletSubcommands::try_parse_from([
            "foundry-cli",
            "sign-auth",
            "0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF",
            "--nonce",
            "42",
            "--self-broadcast",
        ]);
        assert!(
            result.is_err(),
            "expected error when both --nonce and --self-broadcast are provided"
        );
    }
}
