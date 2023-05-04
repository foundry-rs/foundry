//! cast wallet subcommand

pub mod vanity;

use crate::{
    cmd::{cast::wallet::vanity::VanityArgs, Cmd},
    opts::{SignType, Wallet},
};
use cast::SimpleCast;
use clap::Parser;
use ethers::{
    core::rand::thread_rng,
    signers::{LocalWallet, Signer},
    types::{transaction::eip712::TypedData, Address, Signature},
};
use eyre::Context;
use std::{fs::File, io::BufReader};

/// CLI arguments for `cast send`.
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
        #[clap(subcommand)]
        command: Option<SignType>,
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
            WalletSubcommands::Sign { command, wallet } => {
                let wallet = wallet.signer(0).await?;
                match command {
                    Some(SignType::Message { message }) => {
                        let sig = match message.strip_prefix("0x") {
                            Some(data) => {
                                let data_bytes: Vec<u8> = hex::decode(data)
                                    .wrap_err("Could not decode 0x-prefixed string.")?;
                                wallet.sign_message(data_bytes).await?
                            }
                            None => wallet.sign_message(message).await?,
                        };
                        println!("0x{sig}");
                    }
                    Some(SignType::TypedData { from_file, data }) => {
                        let typed_data: TypedData = if from_file {
                            // data is a file name, read json from file
                            let file = File::open(&data)?;
                            let reader = BufReader::new(file);
                            serde_json::from_reader(reader)?
                        } else {
                            // data is a json string
                            serde_json::from_str(&data)?
                        };
                        let sig = wallet.sign_typed_data(&typed_data).await?;
                        println!("0x{sig}");
                    }
                    None => {
                        println!("No subcommand provided. Please provide a subcommand for the type of data to sign.")
                    }
                }
            }
            WalletSubcommands::Verify { message, signature, address } => {
                match signature.verify(message, address) {
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
}
