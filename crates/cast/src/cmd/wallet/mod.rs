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
use foundry_cli::{
    json::{print_json_success, print_scalar},
    opts::RpcOpts,
    utils,
    utils::LoadConfig,
};
use foundry_common::{errors::FsPathError, fs, sh_println, shell};
use foundry_config::Config;
use foundry_wallets::{BrowserWalletOpts, RawWalletOpts, WalletOpts, WalletSigner};
use rand_08::thread_rng;
use serde_json::json;
use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
};
use yansi::Paint;

pub mod vanity;
use vanity::VanityArgs;

pub mod list;
use list::ListArgs;

mod process_tree;

pub mod session;
use session::SessionArgs;

mod touch_id;
use touch_id::TouchIdArgs;

/// CLI arguments for `cast wallet`.
#[derive(Debug, Parser)]
pub enum WalletSubcommands {
    /// Create a new random keypair
    ///
    /// Examples:
    /// - cast wallet new (print a new private key and address)
    /// - cast wallet new ~/.foundry/keystores dev (save to an encrypted keystore)
    #[command(verbatim_doc_comment, visible_alias = "n")]
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

        /// Overwrite existing keystore files without prompting.
        #[arg(long)]
        force: bool,

        /// Enroll the keystore for Touch ID-assisted authentication on macOS.
        ///
        /// The macOS login password and explicit keystore passwords remain available.
        #[arg(long, hide = !cfg!(all(target_os = "macos", feature = "touch-id")))]
        touch_id: bool,
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

        #[command(flatten)]
        browser: BrowserWalletOpts,
    },

    /// Derive accounts from a mnemonic
    ///
    /// Examples:
    /// - cast wallet derive "test test test test test test test test test test test junk"
    /// - cast wallet derive "$MNEMONIC" --accounts 5
    #[command(verbatim_doc_comment, visible_alias = "d")]
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

    /// Sign a message or typed data
    ///
    /// Examples:
    /// - cast wallet sign "hello" --account dev
    /// - cast wallet sign "hello" --private-key $PK
    /// - cast wallet sign --data --from-file typed_data.json --ledger
    #[command(verbatim_doc_comment, visible_alias = "s")]
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

        #[command(flatten)]
        browser: BrowserWalletOpts,
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

        /// Skip the confirmation prompt for wildcard chain authorizations.
        #[arg(long)]
        force: bool,

        /// If set, indicates the authorization will be broadcast by the signing account itself.
        /// This means the nonce used will be the current nonce + 1 (to account for the
        /// transaction that will include this authorization).
        #[arg(long, conflicts_with = "nonce")]
        self_broadcast: bool,

        #[command(flatten)]
        wallet: WalletOpts,
    },

    /// Verify the signature of a message
    ///
    /// Examples:
    /// - cast wallet verify --address $ADDRESS "hello" $SIGNATURE
    /// - cast wallet verify --address $ADDRESS --no-hash $HASH $SIGNATURE
    #[command(verbatim_doc_comment, visible_alias = "v")]
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

    /// Import a private key into an encrypted keystore
    ///
    /// Examples:
    /// - cast wallet import dev --interactive (prompt for the private key)
    /// - cast wallet import dev --private-key $PK
    /// - cast wallet import dev --mnemonic "$MNEMONIC" --mnemonic-index 1
    #[command(verbatim_doc_comment, visible_alias = "i")]
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
        /// Enroll the keystore for Touch ID-assisted authentication on macOS.
        ///
        /// The macOS login password and explicit keystore passwords remain available.
        #[arg(long, hide = !cfg!(all(target_os = "macos", feature = "touch-id")))]
        touch_id: bool,
        #[command(flatten)]
        raw_wallet_options: RawWalletOpts,
    },

    /// List all the accounts in the keystore default directory
    #[command(visible_alias = "ls")]
    List(ListArgs),

    /// Manage temporary Tempo wallet sessions.
    Session(SessionArgs),

    /// Manage Touch ID enrollment for encrypted keystores.
    TouchId(TouchIdArgs),

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
    ///
    /// Examples:
    /// - cast wallet private-key "test test test test test test test test test test test junk"
    /// - cast wallet private-key "$MNEMONIC" 1 (derive the key at index 1)
    /// - cast wallet private-key "$MNEMONIC" "m/44'/60'/0'/0/1" (use a custom path)
    #[command(verbatim_doc_comment, name = "private-key", visible_alias = "pk", aliases = &["derive-private-key", "--derive-private-key"])]
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
            Self::New {
                path,
                account_name,
                unsafe_password,
                number,
                password,
                force,
                touch_id,
            } => {
                ensure_touch_id_available(touch_id)?;
                if let Some(name) = &account_name {
                    ensure_account_name_available(name)?;
                }
                let mut rng = thread_rng();

                let mut json_values = shell::is_json().then(std::vec::Vec::new);

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
                } else if unsafe_password.is_some() || password || touch_id {
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

                        if touch_id {
                            ensure_touch_id_sidecars_available(
                                &path,
                                account_name.as_deref(),
                                number,
                            )?;
                        }

                        // Prevent accidental overwriting: check all target files upfront
                        if !force && let Some(ref acc_name) = account_name {
                            let mut existing_files = Vec::new();

                            for i in 0..number {
                                let name = indexed_account_name(acc_name, number, i);
                                let file_path = path.join(&name);
                                if file_path.exists() {
                                    existing_files.push(name);
                                }
                            }

                            if !existing_files.is_empty() {
                                sh_eprintln!("The following keystore file(s) already exist:")?;
                                for file in &existing_files {
                                    sh_eprintln!("   - {file}")?;
                                }
                                sh_eprint!(
                                    "\nDo you want to overwrite all {} file(s)? [y/N]: ",
                                    existing_files.len()
                                )?;
                                std::io::stderr().flush()?;

                                let mut input = String::new();
                                std::io::stdin().read_line(&mut input)?;

                                if !input.trim().eq_ignore_ascii_case("y") {
                                    eyre::bail!("Operation cancelled. No keystores were modified.");
                                }
                            }
                        }
                        for i in 0..number {
                            let account_name_ref = account_name
                                .as_deref()
                                .map(|name| indexed_account_name(name, number, i));

                            let (wallet, uuid) = PrivateKeySigner::new_keystore(
                                &path,
                                &mut rng,
                                &password,
                                account_name_ref.as_deref(),
                            )?;
                            let identifier = account_name_ref.as_deref().unwrap_or(&uuid);
                            let keystore_path = path.join(identifier);

                            #[cfg(all(target_os = "macos", feature = "touch-id"))]
                            if touch_id {
                                ensure_touch_id_sidecar_available(&keystore_path).map_err(|e| {
                                    eyre::eyre!(
                                        "keystore was created at {}, but Touch ID enrollment preflight failed: {e}. The sidecar was left untouched and must be resolved manually before password-prompt fallback is reliable",
                                        keystore_path.display()
                                    )
                                })?;
                                if let Err(enrollment_error) = foundry_wallets::touch_id::enroll(
                                    &keystore_path,
                                    &password,
                                    foundry_wallets::touch_id::Policy::default(),
                                ) {
                                    let completed_action = if i == 0 {
                                        format!(
                                            "keystore was created at {}",
                                            keystore_path.display()
                                        )
                                    } else {
                                        format!(
                                            "keystore was created at {} (earlier batch keystores were not rolled back)",
                                            keystore_path.display()
                                        )
                                    };
                                    return Err(touch_id_enrollment_failure(
                                        &keystore_path,
                                        &completed_action,
                                        enrollment_error,
                                    ));
                                }
                            }

                            if let Some(json) = json_values.as_mut() {
                                let mut result = json!({
                                    "address": wallet.address().to_checksum(None),
                                    "public_key": format!("0x{}", hex::encode(wallet.public_key())),
                                    "path": format!("{}", keystore_path.display()),
                                });
                                if touch_id {
                                    result["touch_id"] = json!(true);
                                }
                                json.push(result);
                            } else {
                                sh_status!(
                                    "Created new encrypted keystore file: {}",
                                    keystore_path.display()
                                )?;
                                if touch_id {
                                    sh_status!(
                                        "Touch ID-assisted unlock enrolled; password-based unlock remains available."
                                    )?;
                                }
                                sh_status!("Address:    {}", wallet.address().to_checksum(None))?;
                                if shell::verbosity() > 0 {
                                    sh_status!(
                                        "Public key: 0x{}",
                                        hex::encode(wallet.public_key())
                                    )?;
                                }
                                // The machine-readable stdout record duplicates the prose above
                                // when stdout is an interactive terminal.
                                if !shell::is_out_tty() {
                                    sh_println!("{}", wallet.address().to_checksum(None))?;
                                }
                            }
                        }
                    }
                    None => {
                        for _ in 0..number {
                            let wallet = PrivateKeySigner::random_with(&mut rng);

                            if let Some(json) = json_values.as_mut() {
                                json.push(json!({
                                    "address": wallet.address().to_checksum(None),
                                    "public_key": format!("0x{}", hex::encode(wallet.public_key())),
                                    "private_key": format!("0x{}", hex::encode(wallet.credential().to_bytes())),
                                }));
                            } else {
                                sh_status!("Successfully created new keypair.")?;
                                sh_status!("Address:     {}", wallet.address().to_checksum(None))?;
                                if shell::verbosity() > 0 {
                                    sh_status!(
                                        "Public key:  0x{}",
                                        hex::encode(wallet.public_key())
                                    )?;
                                }
                                sh_status!(
                                    "Private key: 0x{}",
                                    hex::encode(wallet.credential().to_bytes())
                                )?;
                                // The machine-readable stdout record duplicates the prose above
                                // when stdout is an interactive terminal.
                                if !shell::is_out_tty() {
                                    sh_println!(
                                        "{}\t0x{}",
                                        wallet.address().to_checksum(None),
                                        hex::encode(wallet.credential().to_bytes())
                                    )?;
                                }
                            }
                        }
                    }
                }

                if let Some(json) = json_values {
                    print_json_success(json)?;
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
                    print_json_success(json!({
                        "mnemonic": phrase,
                        "accounts": accounts,
                    }))?;
                }
            }
            Self::Vanity(cmd) => {
                cmd.run()?;
            }
            Self::Address { wallet, browser, private_key_override } => {
                let addr = if let Some(pk) = private_key_override {
                    WalletOpts {
                        raw: RawWalletOpts { private_key: Some(pk), ..Default::default() },
                        ..Default::default()
                    }
                    .signer()
                    .await?
                    .address()
                } else if let Some(browser) = browser.run::<alloy_network::Ethereum>().await? {
                    browser.address()
                } else {
                    wallet.signer().await?.address()
                };
                print_scalar(addr.to_checksum(None))?;
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
                                        "address": address,
                                        "private_key": format!("0x{}", private_key),
                                    }));
                                } else {
                                    accounts_json.as_array_mut().unwrap().push(json!({
                                        "address": address
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
                        _ => {
                            eyre::bail!("Only local wallets are supported by this command");
                        }
                    }
                }

                if format_json {
                    print_json_success(accounts_json)?;
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
                    _ => {
                        eyre::bail!("Only local wallets are supported by this command");
                    }
                };

                print_scalar(format!("0x{}", hex::encode(public_key)))?;
            }
            Self::Sign { message, data, from_file, no_hash, wallet, browser } => {
                if browser.browser && no_hash {
                    eyre::bail!("Raw hash signing is not supported with a browser wallet");
                }

                let typed_data = if data {
                    let typed_data: TypedData = if from_file {
                        // data is a file name, read json from file
                        foundry_common::fs::read_json_file(message.as_ref())?
                    } else {
                        // data is a json string
                        serde_json::from_str(&message)?
                    };
                    Some(typed_data)
                } else {
                    None
                };

                let (sig, address) =
                    if let Some(browser) = browser.run::<alloy_network::Ethereum>().await? {
                        let sig = if let Some(typed_data) = &typed_data {
                            browser.sign_dynamic_typed_data(typed_data).await?
                        } else {
                            browser.sign_message(&Self::hex_str_to_bytes(&message)?).await?
                        };
                        (sig, browser.address())
                    } else {
                        let wallet = wallet.signer().await?;
                        let sig = if let Some(typed_data) = &typed_data {
                            wallet.sign_dynamic_typed_data(typed_data).await?
                        } else if no_hash {
                            wallet.sign_hash(&hex::decode(&message)?[..].try_into()?).await?
                        } else {
                            wallet.sign_message(&Self::hex_str_to_bytes(&message)?).await?
                        };
                        (sig, wallet.address())
                    };

                if shell::verbosity() > 0 {
                    if shell::is_json() {
                        print_json_success(json!({
                            "message": message,
                            "address": address,
                            "signature": hex::encode(sig.as_bytes()),
                        }))?;
                    } else {
                        sh_status!("Successfully signed!")?;
                        sh_status!("   Message: {message}")?;
                        sh_status!("   Address: {address}")?;
                        sh_println!("0x{}", hex::encode(sig.as_bytes()))?;
                    }
                } else {
                    print_scalar(format!("0x{}", hex::encode(sig.as_bytes())))?;
                }
            }
            Self::SignAuth { rpc, nonce, chain, force, wallet, address, self_broadcast } => {
                let provider = utils::get_provider(&rpc.load_config()?)?;
                let chain_id = if let Some(chain) = chain {
                    chain.id()
                } else {
                    provider.get_chain_id().await?
                };
                if chain_id == 0 && !force {
                    sh_warn!(
                        "Chain ID 0 creates an EIP-7702 authorization that is valid on every chain."
                    )?;
                    let response: String = foundry_common::prompt!("\nContinue anyway? [y/N] ")?;
                    if !matches!(response.trim(), "y" | "Y") {
                        sh_status!("Aborted.")?;
                        return Ok(());
                    }
                }

                let wallet = wallet.signer().await?;
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
                let auth = Authorization { chain_id: U256::from(chain_id), address, nonce };
                let signature = wallet.sign_hash(&auth.signature_hash()).await?;
                let auth = auth.into_signed(signature);

                if shell::verbosity() > 0 {
                    if shell::is_json() {
                        print_json_success(json!({
                            "nonce": nonce,
                            "chain_id": chain_id,
                            "address": wallet.address(),
                            "signature": hex::encode_prefixed(alloy_rlp::encode(&auth)),
                        }))?;
                    } else {
                        sh_status!("Successfully signed!")?;
                        sh_status!("   Nonce: {nonce}")?;
                        sh_status!("   Chain ID: {chain_id}")?;
                        sh_status!("   Address: {}", wallet.address())?;
                        sh_println!("{}", hex::encode_prefixed(alloy_rlp::encode(&auth)))?;
                    }
                } else {
                    print_scalar(hex::encode_prefixed(alloy_rlp::encode(&auth)))?;
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
                    if shell::is_json() {
                        print_json_success(json!({"address": address, "result": true}))?;
                    } else {
                        sh_println!(
                            "Validation succeeded. Address {address} signed this message."
                        )?;
                    }
                } else {
                    eyre::bail!("Validation failed. Address {address} did not sign this message.");
                }
            }
            Self::Import {
                account_name,
                keystore_dir,
                unsafe_password,
                touch_id,
                raw_wallet_options,
            } => {
                ensure_touch_id_available(touch_id)?;
                ensure_account_name_available(&account_name)?;
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
                if touch_id {
                    ensure_touch_id_sidecar_available(&keystore_path)?;
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
                    &password,
                    Some(&account_name),
                )?;
                let address = wallet.address();

                #[cfg(all(target_os = "macos", feature = "touch-id"))]
                if touch_id {
                    ensure_touch_id_sidecar_available(&keystore_path).map_err(|e| {
                        eyre::eyre!(
                            "keystore was imported at {}, but Touch ID enrollment preflight failed: {e}. The sidecar was left untouched and must be resolved manually before password-prompt fallback is reliable",
                            keystore_path.display()
                        )
                    })?;
                    if let Err(enrollment_error) = foundry_wallets::touch_id::enroll(
                        &keystore_path,
                        &password,
                        foundry_wallets::touch_id::Policy::default(),
                    ) {
                        return Err(touch_id_enrollment_failure(
                            &keystore_path,
                            &format!("keystore was imported at {}", keystore_path.display()),
                            enrollment_error,
                        ));
                    }
                }

                if shell::is_json() {
                    let mut result = json!({"account": account_name, "address": address});
                    if touch_id {
                        result["touch_id"] = json!(true);
                    }
                    print_json_success(result)?;
                } else {
                    sh_println!(
                        "{}",
                        format!(
                            "`{account_name}` keystore was saved successfully. Address: {address:?}"
                        )
                        .green()
                    )?;
                    if touch_id {
                        sh_status!(
                            "Touch ID-assisted unlock enrolled; password-based unlock remains available."
                        )?;
                    }
                }
            }
            Self::List(cmd) => {
                cmd.run().await?;
            }
            Self::Session(args) => {
                args.run().await?;
            }
            Self::TouchId(args) => {
                args.run()?;
            }
            Self::Remove { name, dir, unsafe_password } => {
                ensure_account_name_available(&name)?;
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

                remove_touch_id_sidecar(&keystore_path)?;

                std::fs::remove_file(&keystore_path).wrap_err_with(|| {
                    format!("Failed to remove keystore file at {}", keystore_path.display())
                })?;

                if shell::is_json() {
                    print_json_success(json!({"account": name, "removed": true}))?;
                } else {
                    sh_println!(
                        "{}",
                        format!("`{name}` keystore was removed successfully.").green()
                    )?;
                }
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
                        let private_key =
                            format!("0x{}", hex::encode(wallet.credential().to_bytes()));
                        if shell::verbosity() > 0 {
                            if shell::is_json() {
                                print_json_success(json!({
                                    "address": wallet.address(),
                                    "private_key": private_key,
                                }))?;
                            } else {
                                sh_println!("Address:     {}", wallet.address())?;
                                sh_println!("Private key: {private_key}")?;
                            }
                        } else {
                            print_scalar(private_key)?;
                        }
                    }
                    _ => {
                        eyre::bail!("Only local wallets are supported by this command.");
                    }
                }
            }
            Self::DecryptKeystore { account_name, keystore_dir, unsafe_password } => {
                ensure_account_name_available(&account_name)?;
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
                if shell::is_json() {
                    print_json_success(
                        json!({"account": account_name, "private_key": private_key}),
                    )?;
                } else {
                    sh_println!(
                        "{}",
                        format!("{account_name}'s private key is: {private_key}").green()
                    )?;
                }
            }
            Self::ChangePassword {
                account_name,
                keystore_dir,
                unsafe_password,
                unsafe_new_password,
            } => {
                ensure_account_name_available(&account_name)?;
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

                let sidecar = touch_id_sidecar_path(&keypath);

                let touch_id_enrolled = match touch_id_sidecar_state(&sidecar)? {
                    TouchIdSidecarState::Missing => false,
                    TouchIdSidecarState::Recognized => true,

                    TouchIdSidecarState::Keystore => {
                        eyre::bail!(
                            "refusing to change the password because {} is an existing keystore",
                            sidecar.display()
                        );
                    }

                    TouchIdSidecarState::Unknown => {
                        #[cfg(all(target_os = "macos", feature = "touch-id"))]
                        {
                            // Preserve useful structured errors such as UnsupportedVersion.
                            if let Err(error) = foundry_wallets::touch_id::policy(&keypath) {
                                return Err(error.into());
                            }
                        }

                        // Never continue after an Unknown classification, even if another
                        // parser happens to accept the file.
                        eyre::bail!(
                            "refusing to change the password because {} exists and is not a recognized Touch ID sidecar",
                            sidecar.display()
                        );
                    }
                };

                #[cfg(all(target_os = "macos", feature = "touch-id"))]
                let touch_id_policy = touch_id_enrolled
                    .then(|| foundry_wallets::touch_id::policy(&keypath))
                    .transpose()?;

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
                    &new_password,
                    Some(&account_name),
                )?;

                #[cfg(all(target_os = "macos", feature = "touch-id"))]
                if let Some(policy) = touch_id_policy
                    && let Err(enrollment_error) =
                        foundry_wallets::touch_id::enroll(&keypath, &new_password, policy)
                {
                    return Err(touch_id_enrollment_failure(
                        &keypath,
                        &format!(
                            "password for keystore `{account_name}` was changed at {}",
                            keypath.display()
                        ),
                        enrollment_error,
                    ));
                }

                #[cfg(not(all(target_os = "macos", feature = "touch-id")))]
                if touch_id_enrolled {
                    match remove_touch_id_sidecar(&keypath) {
                        Ok(true) => {
                            sh_warn!(
                                "Removed the stale Touch ID enrollment after changing the password"
                            )?;
                        }
                        Ok(false) => {}
                        Err(cleanup_error) => {
                            eyre::bail!(
                                "password changed, but Touch ID sidecar cleanup failed: {cleanup_error}. The new password is valid; remove {} manually",
                                touch_id_sidecar_path(&keypath).display()
                            );
                        }
                    }
                }

                let address = wallet.address();
                if shell::is_json() {
                    print_json_success(json!({"account": account_name, "address": address}))?;
                } else {
                    sh_println!(
                        "{}",
                        format!(
                            "Password for keystore `{account_name}` was changed successfully. Address: {address:?}"
                        )
                        .green()
                    )?;
                }
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

fn ensure_touch_id_available(touch_id: bool) -> Result<()> {
    if !touch_id {
        return Ok(());
    }

    #[cfg(all(target_os = "macos", feature = "touch-id"))]
    {
        if !foundry_wallets::touch_id::is_available() {
            eyre::bail!("Touch ID is unavailable on this Mac");
        }
        Ok(())
    }

    #[cfg(not(all(target_os = "macos", feature = "touch-id")))]
    eyre::bail!("`--touch-id` requires macOS and a cast build with the `touch-id` feature");
}

const TOUCH_ID_SIDECAR_SUFFIX: &str = ".touchid";

fn ensure_account_name_available(name: &str) -> Result<()> {
    let file_name = Path::new(name).file_name().and_then(|s| s.to_str());
    if name.is_empty() || name.contains('\\') || file_name != Some(name) {
        eyre::bail!("account name must be a single path segment");
    }
    if name.ends_with(TOUCH_ID_SIDECAR_SUFFIX) {
        eyre::bail!("account names ending in `{TOUCH_ID_SIDECAR_SUFFIX}` are reserved");
    }
    Ok(())
}

fn touch_id_sidecar_path(keystore_path: &Path) -> PathBuf {
    let mut path = OsString::from(keystore_path.as_os_str());
    path.push(TOUCH_ID_SIDECAR_SUFFIX);
    path.into()
}

fn is_not_found(error: &FsPathError) -> bool {
    matches!(error, FsPathError::Read { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
}

/// Classification of a file at a `.touchid` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TouchIdSidecarState {
    /// No filesystem entry exists at the path.
    Missing,
    /// The file strictly matches the currently supported Touch ID sidecar schema.
    Recognized,
    /// The file is an Ethereum keystore.
    Keystore,
    /// Anything else: malformed JSON, empty object, array, unrelated object,
    /// unsupported sidecar version, unknown policy, invalid hex, empty payload,
    /// truncated sealed payload, invalid X9.63 prefix, or future sidecar format.
    Unknown,
}

/// The only sidecar version this Cast release understands.
const TOUCH_ID_SIDECAR_VERSION: u32 = 1;

/// Minimum encoded ciphertext payload:
/// 65-byte P-256 X9.63 public key + 12-byte ChaChaPoly nonce + 16-byte tag.
///
/// The encrypted password itself may be empty, so 93 bytes is the true minimum.
const TOUCH_ID_SEALED_PASSWORD_MIN_LEN: usize = 65 + 12 + 16;

/// X9.63 prefix for an uncompressed P-256 public key.
const TOUCH_ID_X963_UNCOMPRESSED_PREFIX: u8 = 0x04;

/// Strict deserialization-only representation of the persisted sidecar format.
///
/// Uses `deny_unknown_fields` so that any unrecognized field (e.g. from a
/// future sidecar version) causes a parse failure, which maps to `Unknown`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct TouchIdSidecarWire {
    version: u32,
    policy: TouchIdPolicyWire,
    se_key: String,
    sealed_password: String,
}

/// The policy values currently recognised by this Cast release.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TouchIdPolicyWire {
    UserPresence,
    CurrentBiometry,
}

impl TouchIdPolicyWire {
    const fn as_str(self) -> &'static str {
        match self {
            Self::UserPresence => "user-presence",
            Self::CurrentBiometry => "current-biometry",
        }
    }
}

/// Validates that `wire` contains plausible payload bytes:
/// - `se_key` is valid hex and non-empty.
/// - `sealed_password` is valid hex, at least 93 decoded bytes, and starts with 0x04.
fn has_valid_touch_id_payload(wire: &TouchIdSidecarWire) -> bool {
    let Ok(se_key) = hex::decode(&wire.se_key) else {
        return false;
    };

    if se_key.is_empty() {
        return false;
    }

    let Ok(sealed_password) = hex::decode(&wire.sealed_password) else {
        return false;
    };

    sealed_password.len() >= TOUCH_ID_SEALED_PASSWORD_MIN_LEN
        && sealed_password.first().copied() == Some(TOUCH_ID_X963_UNCOMPRESSED_PREFIX)
}

/// Returns the state of the file at `path` with respect to the Touch ID sidecar schema.
///
/// Classification order:
/// 1. Not found → `Missing`
/// 2. Has both `version` and `crypto`/`Crypto` fields → `Keystore`
/// 3. Parses strictly as a v1 Touch ID sidecar + plausible payload → `Recognized`
/// 4. Everything else → `Unknown`
fn touch_id_sidecar_state(path: &Path) -> Result<TouchIdSidecarState> {
    let value = match fs::read_json_file::<serde_json::Value>(path) {
        Ok(v) => v,
        Err(e) if is_not_found(&e) => return Ok(TouchIdSidecarState::Missing),
        Err(e) => return Err(e.into()),
    };

    // Check for Ethereum keystore before attempting sidecar parse.
    // Preserves both lowercase and uppercase `crypto` field variants.
    if value.get("version").is_some()
        && (value.get("crypto").is_some() || value.get("Crypto").is_some())
    {
        return Ok(TouchIdSidecarState::Keystore);
    }

    // Attempt strict sidecar parse. Any missing/extra field, unsupported
    // version/policy, or invalid payload maps to `Unknown` rather than `Recognized`.
    match serde_json::from_value::<TouchIdSidecarWire>(value) {
        Ok(wire)
            if wire.version == TOUCH_ID_SIDECAR_VERSION && has_valid_touch_id_payload(&wire) =>
        {
            Ok(TouchIdSidecarState::Recognized)
        }
        _ => Ok(TouchIdSidecarState::Unknown),
    }
}

fn touch_id_sidecar_policy(path: &Path) -> Result<TouchIdPolicyWire> {
    let value = fs::read_json_file::<serde_json::Value>(path)?;
    let wire = serde_json::from_value::<TouchIdSidecarWire>(value)
        .wrap_err_with(|| format!("failed to parse Touch ID sidecar at {}", path.display()))?;
    if wire.version != TOUCH_ID_SIDECAR_VERSION || !has_valid_touch_id_payload(&wire) {
        eyre::bail!("{} is not a recognized Touch ID sidecar", path.display());
    }
    Ok(wire.policy)
}

fn is_touch_id_sidecar(path: &Path) -> Result<bool> {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(false);
    };
    if !name.ends_with(TOUCH_ID_SIDECAR_SUFFIX) {
        return Ok(false);
    }
    Ok(matches!(touch_id_sidecar_state(path)?, TouchIdSidecarState::Recognized))
}

fn ensure_touch_id_sidecar_available(keystore_path: &Path) -> Result<()> {
    let sidecar = touch_id_sidecar_path(keystore_path);
    match touch_id_sidecar_state(&sidecar)? {
        TouchIdSidecarState::Missing | TouchIdSidecarState::Recognized => Ok(()),
        TouchIdSidecarState::Keystore => {
            eyre::bail!(
                "refusing Touch ID enrollment because {} is an existing keystore",
                sidecar.display()
            );
        }
        TouchIdSidecarState::Unknown => {
            eyre::bail!(
                "refusing Touch ID enrollment because {} already exists and is not a recognized Touch ID sidecar",
                sidecar.display()
            );
        }
    }
}

fn indexed_account_name(base: &str, number: u32, index: u32) -> String {
    if number == 1 { base.to_string() } else { format!("{base}_{}", index + 1) }
}

fn ensure_touch_id_sidecars_available(
    dir: &Path,
    account_name: Option<&str>,
    number: u32,
) -> Result<()> {
    let Some(account_name) = account_name else { return Ok(()) };
    for index in 0..number {
        ensure_touch_id_sidecar_available(&dir.join(indexed_account_name(
            account_name,
            number,
            index,
        )))?;
    }
    Ok(())
}

fn remove_touch_id_sidecar(keystore_path: &Path) -> Result<bool> {
    let sidecar = touch_id_sidecar_path(keystore_path);
    match touch_id_sidecar_state(&sidecar)? {
        TouchIdSidecarState::Missing => Ok(false),
        TouchIdSidecarState::Recognized => match std::fs::remove_file(&sidecar) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).wrap_err_with(|| {
                format!("Failed to remove Touch ID sidecar at {}", sidecar.display())
            }),
        },
        TouchIdSidecarState::Keystore => {
            eyre::bail!("refusing to remove existing keystore at {}", sidecar.display());
        }
        TouchIdSidecarState::Unknown => {
            eyre::bail!(
                "refusing to remove {} because it is not a recognized Touch ID sidecar",
                sidecar.display()
            );
        }
    }
}

#[cfg(all(target_os = "macos", feature = "touch-id"))]
fn touch_id_enrollment_failure(
    keystore_path: &Path,
    completed_action: &str,
    enrollment_error: impl std::fmt::Display,
) -> eyre::Report {
    match remove_touch_id_sidecar(keystore_path) {
        Ok(true) => eyre::eyre!(
            "{completed_action}, but Touch ID enrollment failed: {enrollment_error}. The stale Touch ID sidecar was removed; password-prompt fallback remains available"
        ),
        Ok(false) => eyre::eyre!(
            "{completed_action}, but Touch ID enrollment failed: {enrollment_error}. No stale Touch ID sidecar remained; password-prompt fallback remains available"
        ),
        Err(cleanup_error) => eyre::eyre!(
            "{completed_action}, but Touch ID enrollment failed: {enrollment_error}. The stale sidecar could not be removed: {cleanup_error}. Remove {} manually before password-prompt fallback is possible",
            touch_id_sidecar_path(keystore_path).display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{session::SessionSubcommands, *};
    use alloy_primitives::{address, keccak256};
    use std::str::FromStr;

    // ── Touch ID sidecar classification ────────────────────────────────────────

    fn valid_sealed_password_hex() -> String {
        format!("04{}", "00".repeat(TOUCH_ID_SEALED_PASSWORD_MIN_LEN - 1))
    }

    fn touch_id_sidecar_json(
        version: u32,
        policy: &str,
        se_key: &str,
        sealed_password: &str,
    ) -> String {
        serde_json::json!({
            "version": version,
            "policy": policy,
            "se_key": se_key,
            "sealed_password": sealed_password,
        })
        .to_string()
    }

    fn valid_touch_id_sidecar_json(policy: &str) -> String {
        let sealed_password = valid_sealed_password_hex();
        touch_id_sidecar_json(TOUCH_ID_SIDECAR_VERSION, policy, "aa", &sealed_password)
    }

    /// Returns a temp dir and the path `<dir>/account.touchid`.
    fn setup_sidecar_path() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("account.touchid");
        (dir, path)
    }

    /// Helper: write content to `path` and return the path.
    fn write<'a>(path: &'a std::path::Path, content: &str) -> &'a std::path::Path {
        std::fs::write(path, content).unwrap();
        path
    }

    // ── is_touch_id_sidecar ────────────────────────────────────────────────────

    #[test]
    fn recognized_sidecar_user_presence() {
        let (_dir, p) = setup_sidecar_path();
        write(&p, &valid_touch_id_sidecar_json("user-presence"));
        assert!(is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Recognized);
    }

    #[test]
    fn recognized_sidecar_current_biometry() {
        let (_dir, p) = setup_sidecar_path();
        write(&p, &valid_touch_id_sidecar_json("current-biometry"));
        assert!(is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Recognized);
    }

    #[test]
    fn empty_object_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        write(&p, "{}");
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn array_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        write(&p, "[]");
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn unrelated_object_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        write(&p, r#"{"application":"unrelated"}"#);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn unknown_version_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        let content = touch_id_sidecar_json(2, "user-presence", "aa", &valid_sealed_password_hex());
        write(&p, &content);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn unknown_field_is_unknown_due_to_deny_unknown_fields() {
        let (_dir, p) = setup_sidecar_path();
        let json = serde_json::json!({
            "version": 1,
            "policy": "user-presence",
            "se_key": "aa",
            "sealed_password": valid_sealed_password_hex(),
            "future_field": true
        })
        .to_string();
        write(&p, &json);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn unknown_policy_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        let content = touch_id_sidecar_json(1, "future-policy", "aa", &valid_sealed_password_hex());
        write(&p, &content);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn empty_se_key_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        let content = touch_id_sidecar_json(1, "user-presence", "", &valid_sealed_password_hex());
        write(&p, &content);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn non_hex_se_key_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        let content = touch_id_sidecar_json(1, "user-presence", "zz", &valid_sealed_password_hex());
        write(&p, &content);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn empty_sealed_password_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        let content = touch_id_sidecar_json(1, "user-presence", "aa", "");
        write(&p, &content);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn non_hex_sealed_password_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        let content = touch_id_sidecar_json(1, "user-presence", "aa", "zz");
        write(&p, &content);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn truncated_sealed_password_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        let truncated_sealed = format!("04{}", "00".repeat(TOUCH_ID_SEALED_PASSWORD_MIN_LEN - 2));
        let content = touch_id_sidecar_json(1, "user-presence", "aa", &truncated_sealed);
        write(&p, &content);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn sealed_password_without_uncompressed_point_prefix_is_unknown() {
        let (_dir, p) = setup_sidecar_path();
        let invalid_prefix_sealed =
            format!("03{}", "00".repeat(TOUCH_ID_SEALED_PASSWORD_MIN_LEN - 1));
        let content = touch_id_sidecar_json(1, "user-presence", "aa", &invalid_prefix_sealed);
        write(&p, &content);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Unknown);
    }

    #[test]
    fn minimum_valid_sealed_password_is_recognized() {
        let (_dir, p) = setup_sidecar_path();
        let exact_min_sealed = format!("04{}", "00".repeat(TOUCH_ID_SEALED_PASSWORD_MIN_LEN - 1));
        let content = touch_id_sidecar_json(1, "user-presence", "aa", &exact_min_sealed);
        write(&p, &content);
        assert!(is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Recognized);
    }

    #[test]
    fn invalid_payload_sidecars_are_preserved() {
        let valid_sealed = valid_sealed_password_hex();
        let truncated_sealed = format!("04{}", "00".repeat(TOUCH_ID_SEALED_PASSWORD_MIN_LEN - 2));
        let bad_prefix_sealed = format!("03{}", "00".repeat(TOUCH_ID_SEALED_PASSWORD_MIN_LEN - 1));

        let invalid_fixtures = [
            touch_id_sidecar_json(1, "user-presence", "", &valid_sealed),
            touch_id_sidecar_json(1, "user-presence", "zz", &valid_sealed),
            touch_id_sidecar_json(1, "user-presence", "aa", ""),
            touch_id_sidecar_json(1, "user-presence", "aa", "zz"),
            touch_id_sidecar_json(1, "user-presence", "aa", &truncated_sealed),
            touch_id_sidecar_json(1, "user-presence", "aa", &bad_prefix_sealed),
        ];

        for fixture in invalid_fixtures {
            let dir = tempfile::tempdir().unwrap();
            let sidecar = dir.path().join("account.touchid");
            write(&sidecar, &fixture);
            let k_path = keystore_path(dir.path());

            let err = ensure_touch_id_sidecar_available(&k_path).unwrap_err();
            assert!(
                err.to_string().contains("is not a recognized Touch ID sidecar"),
                "unexpected error: {err}"
            );
            assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), fixture);

            let err = remove_touch_id_sidecar(&k_path).unwrap_err();
            assert!(
                err.to_string().contains("is not a recognized Touch ID sidecar"),
                "unexpected error: {err}"
            );
            assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), fixture);
        }
    }

    #[test]
    fn malformed_json_propagates_error() {
        let (_dir, p) = setup_sidecar_path();
        write(&p, "not json");
        // Malformed JSON is an I/O/parse error, not Unknown.
        assert!(is_touch_id_sidecar(&p).is_err());
        assert!(touch_id_sidecar_state(&p).is_err());
    }

    #[test]
    fn keystore_lowercase_crypto_is_keystore() {
        let (_dir, p) = setup_sidecar_path();
        write(&p, r#"{"version":3,"crypto":{}}"#);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Keystore);
    }

    #[test]
    fn keystore_uppercase_crypto_is_keystore() {
        let (_dir, p) = setup_sidecar_path();
        write(&p, r#"{"version":3,"Crypto":{}}"#);
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Keystore);
    }

    #[test]
    fn missing_file_is_missing() {
        let (_dir, p) = setup_sidecar_path();
        // File was never created.
        assert!(!is_touch_id_sidecar(&p).unwrap());
        assert_eq!(touch_id_sidecar_state(&p).unwrap(), TouchIdSidecarState::Missing);
    }

    // ── ensure_touch_id_sidecar_available ─────────────────────────────────────

    /// Writes a recognized sidecar at `<dir>/account.touchid` and calls
    /// `ensure_touch_id_sidecar_available` for `<dir>/account`.
    fn keystore_path(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("account")
    }

    #[test]
    fn enrollment_allows_missing_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        // No sidecar file exists — enrollment must succeed.
        ensure_touch_id_sidecar_available(&keystore_path(dir.path())).unwrap();
    }

    #[test]
    fn enrollment_allows_replacing_recognized_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        write(&sidecar, &valid_touch_id_sidecar_json("user-presence"));
        // Existing recognized sidecar — re-enrollment must succeed.
        ensure_touch_id_sidecar_available(&keystore_path(dir.path())).unwrap();
    }

    #[test]
    fn enrollment_refuses_keystore() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        write(&sidecar, r#"{"version":3,"crypto":{}}"#);
        let err = ensure_touch_id_sidecar_available(&keystore_path(dir.path())).unwrap_err();
        assert!(err.to_string().contains("is an existing keystore"), "unexpected error: {err}");
        // File must be untouched.
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), r#"{"version":3,"crypto":{}}"#);
    }

    #[test]
    fn enrollment_refuses_unknown_file() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        write(&sidecar, r#"{"application":"unrelated"}"#);
        let err = ensure_touch_id_sidecar_available(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("is not a recognized Touch ID sidecar"),
            "unexpected error: {err}"
        );
        // File must be untouched.
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), r#"{"application":"unrelated"}"#);
    }

    #[test]
    fn enrollment_refuses_empty_object() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        write(&sidecar, "{}");
        let err = ensure_touch_id_sidecar_available(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("is not a recognized Touch ID sidecar"),
            "unexpected error: {err}"
        );
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "{}");
    }

    #[test]
    fn enrollment_refuses_array() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        write(&sidecar, "[]");
        let err = ensure_touch_id_sidecar_available(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("is not a recognized Touch ID sidecar"),
            "unexpected error: {err}"
        );
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "[]");
    }

    #[test]
    fn enrollment_refuses_unknown_version() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        let content = touch_id_sidecar_json(2, "user-presence", "aa", &valid_sealed_password_hex());
        write(&sidecar, &content);
        let err = ensure_touch_id_sidecar_available(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("is not a recognized Touch ID sidecar"),
            "unexpected error: {err}"
        );
        // Future sidecar format must not be destroyed.
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), content);
    }

    // ── remove_touch_id_sidecar ────────────────────────────────────────────────

    #[test]
    fn removal_returns_false_for_missing_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let removed = remove_touch_id_sidecar(&keystore_path(dir.path())).unwrap();
        assert!(!removed);
    }

    #[test]
    fn removal_deletes_recognized_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        write(&sidecar, &valid_touch_id_sidecar_json("user-presence"));
        let removed = remove_touch_id_sidecar(&keystore_path(dir.path())).unwrap();
        assert!(removed);
        assert!(!sidecar.exists());
    }

    #[test]
    fn removal_refuses_keystore() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        let content = r#"{"version":3,"crypto":{}}"#;
        write(&sidecar, content);
        let err = remove_touch_id_sidecar(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("refusing to remove existing keystore"),
            "unexpected error: {err}"
        );
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), content);
    }

    #[test]
    fn removal_refuses_empty_object() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        write(&sidecar, "{}");
        let err = remove_touch_id_sidecar(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("is not a recognized Touch ID sidecar"),
            "unexpected error: {err}"
        );
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "{}");
    }

    #[test]
    fn removal_refuses_array() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        write(&sidecar, "[]");
        let err = remove_touch_id_sidecar(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("is not a recognized Touch ID sidecar"),
            "unexpected error: {err}"
        );
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), "[]");
    }

    #[test]
    fn removal_refuses_unrelated_object() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        let content = r#"{"application":"unrelated"}"#;
        write(&sidecar, content);
        let err = remove_touch_id_sidecar(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("is not a recognized Touch ID sidecar"),
            "unexpected error: {err}"
        );
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), content);
    }

    #[test]
    fn removal_refuses_unknown_version() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("account.touchid");
        let content = touch_id_sidecar_json(2, "user-presence", "aa", &valid_sealed_password_hex());
        write(&sidecar, &content);
        let err = remove_touch_id_sidecar(&keystore_path(dir.path())).unwrap_err();
        assert!(
            err.to_string().contains("is not a recognized Touch ID sidecar"),
            "unexpected error: {err}"
        );
        // Future sidecar format must not be destroyed.
        assert_eq!(std::fs::read_to_string(&sidecar).unwrap(), content);
    }

    // ── Wallet listing regression ──────────────────────────────────────────────

    /// Verify that the listing filter uses `!matches!(is_touch_id_sidecar(&path), Ok(true))`:
    /// - recognized v1 sidecars are hidden
    /// - unknown-content `.touchid` files are retained
    /// - unknown-version `.touchid` files are retained
    /// - invalid-payload `.touchid` files are retained
    #[test]
    fn listing_hides_only_recognized_sidecars() {
        // is_touch_id_sidecar returns Ok(true) only for Recognized.
        let dir = tempfile::tempdir().unwrap();

        let recognized = dir.path().join("recognized.touchid");
        write(&recognized, &valid_touch_id_sidecar_json("user-presence"));

        let unknown_content = dir.path().join("unknown_content.touchid");
        write(&unknown_content, r#"{"application":"unrelated"}"#);

        let unknown_version = dir.path().join("unknown_version.touchid");
        write(
            &unknown_version,
            &touch_id_sidecar_json(2, "user-presence", "aa", &valid_sealed_password_hex()),
        );

        let invalid_payload = dir.path().join("invalid_payload.touchid");
        write(&invalid_payload, &touch_id_sidecar_json(1, "user-presence", "aa", "bb"));

        // Recognized sidecar → is_touch_id_sidecar returns Ok(true) → hidden.
        assert!(matches!(is_touch_id_sidecar(&recognized), Ok(true)));
        // Unknown content → Ok(false) → retained by listing.
        assert!(matches!(is_touch_id_sidecar(&unknown_content), Ok(false)));
        // Unknown version → Ok(false) → retained by listing.
        assert!(matches!(is_touch_id_sidecar(&unknown_version), Ok(false)));
        // Invalid payload → Ok(false) → retained by listing.
        assert!(matches!(is_touch_id_sidecar(&invalid_payload), Ok(false)));
    }

    // ── preflights_every_named_touch_id_sidecar (preserved from before) ────────

    #[test]
    fn preflights_every_named_touch_id_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("batch_2.touchid");
        std::fs::write(&sidecar, r#"{"version":3,"crypto":{}}"#).unwrap();

        let error = ensure_touch_id_sidecars_available(dir.path(), Some("batch"), 2).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "refusing Touch ID enrollment because {} is an existing keystore",
                sidecar.display()
            )
        );
    }

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
    fn can_parse_wallet_new_touch_id() {
        let args = WalletSubcommands::parse_from(["foundry-cli", "new", "--touch-id"]);
        match args {
            WalletSubcommands::New { touch_id, .. } => assert!(touch_id),
            _ => panic!("expected WalletSubcommands::New"),
        }
    }

    #[test]
    fn can_parse_wallet_import_touch_id() {
        let args = WalletSubcommands::parse_from([
            "foundry-cli",
            "import",
            "my_account",
            "--touch-id",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
        ]);
        match args {
            WalletSubcommands::Import { touch_id, .. } => assert!(touch_id),
            _ => panic!("expected WalletSubcommands::Import"),
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
    fn can_parse_wallet_session_create() {
        let args = WalletSubcommands::parse_from([
            "foundry-cli",
            "session",
            "create",
            "--root",
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "--chain-id",
            "4217",
            "--expires",
            "10m",
            "--scope",
            "0x20c0000000000000000000000000000000000001:transfer",
            "--spend-limit",
            "PathUSD=0",
            "--private-key",
            "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0",
        ]);

        match args {
            WalletSubcommands::Session(args) => match args.command {
                Some(SessionSubcommands::Create {
                    root_account,
                    chain_id,
                    expires,
                    scope,
                    spend_limits,
                    wallet,
                }) => {
                    assert_eq!(
                        root_account,
                        address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf")
                    );
                    assert_eq!(chain_id, 4217);
                    assert_eq!(expires, 600);
                    assert_eq!(scope.len(), 1);
                    assert_eq!(spend_limits.len(), 1);
                    assert_eq!(
                        wallet.raw.private_key.as_deref(),
                        Some("0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0")
                    );
                }
                _ => panic!("expected WalletSubcommands::Session::Create"),
            },
            _ => panic!("expected WalletSubcommands::Session"),
        }
    }

    #[test]
    fn can_parse_wallet_session_revoke() {
        for (extra_args, expected_local) in [([].as_slice(), false), (["--local"].as_slice(), true)]
        {
            let args = WalletSubcommands::parse_from(
                [
                    "foundry-cli",
                    "session",
                    "revoke",
                    "0x1111111111111111111111111111111111111111111111111111111111111111",
                ]
                .into_iter()
                .chain(extra_args.iter().copied()),
            );

            match args {
                WalletSubcommands::Session(args) => match args.command {
                    Some(SessionSubcommands::Revoke { session_id, local, .. }) => {
                        assert_eq!(session_id, B256::from([0x11; 32]));
                        assert_eq!(local, expected_local);
                    }
                    _ => panic!("expected WalletSubcommands::Session::Revoke"),
                },
                _ => panic!("expected WalletSubcommands::Session"),
            }
        }
    }

    #[test]
    fn can_parse_wallet_session_run_for_command() {
        let args = WalletSubcommands::parse_from([
            "foundry-cli",
            "session",
            "--root",
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "--chain-id",
            "4217",
            "--expires",
            "10m",
            "--target",
            "0x20c0000000000000000000000000000000000001",
            "--selector",
            "transfer(address,uint256)",
            "--spend-limit",
            "PathUSD=0",
            "--for",
            "forge script Deploy --broadcast",
            "--private-key",
            "0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0",
        ]);

        match args {
            WalletSubcommands::Session(args) => {
                assert!(args.command.is_none());
                assert_eq!(
                    args.root_account,
                    Some(address!("0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"))
                );
                assert_eq!(args.send_tx.eth.etherscan.chain.map(|chain| chain.id()), Some(4217));
                assert_eq!(args.expires, Some(600));
                assert_eq!(
                    args.target,
                    Some(address!("0x20c0000000000000000000000000000000000001"))
                );
                assert_eq!(args.selectors.len(), 1);
                assert_eq!(args.spend_limits.len(), 1);
                assert_eq!(args.for_command.as_deref(), Some("forge script Deploy --broadcast"));
                assert_eq!(
                    args.send_tx.eth.wallet.raw.private_key.as_deref(),
                    Some("0x59c6995e998f97a5a004497e5da3b5d2b2b66a87f064d39c44da0b6d6e4f8ff0")
                );
            }
            _ => panic!("expected WalletSubcommands::Session"),
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

    #[test]
    fn rejects_path_keystore_account_name() {
        assert!(ensure_account_name_available("dev").is_ok());
        assert!(ensure_account_name_available("testAccount").is_ok());
        assert!(ensure_account_name_available("../pwned").is_err());
        assert!(ensure_account_name_available("nested/alias").is_err());
        assert!(ensure_account_name_available("foo/../bar").is_err());
        assert!(ensure_account_name_available("..").is_err());
        assert!(ensure_account_name_available(".").is_err());
        assert!(ensure_account_name_available("").is_err());
        assert!(ensure_account_name_available("foo\\bar").is_err());
    }
}
