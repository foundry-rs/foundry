//! cast wallet subcommand

pub mod vanity;

use crate::{
    cmd::{cast::wallet::vanity::VanityArgs, Cmd},
    opts::{Wallet, SignType}
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
    #[clap(name = "new", visible_alias = "n", about = "Create a new random keypair.")]
    New {
        #[clap(
            help = "If provided, then keypair will be written to an encrypted JSON keystore.",
            value_name = "PATH"
        )]
        path: Option<String>,
        #[clap(
            long,
            short,
            help = r#"Deprecated: prompting for a hidden password is now the default.
            Triggers a hidden password prompt for the JSON keystore."#,
            conflicts_with = "unsafe_password",
            requires = "path"
        )]
        password: bool,
        #[clap(
            long,
            help = "Password for the JSON keystore in cleartext. This is UNSAFE to use and we recommend using the --password.",
            requires = "path",
            env = "CAST_PASSWORD",
            value_name = "PASSWORD"
        )]
        unsafe_password: Option<String>,
    },
    #[clap(name = "vanity", visible_alias = "va", about = "Generate a vanity address.")]
    Vanity(VanityArgs),
    #[clap(name = "address", visible_aliases = &["a", "addr"], about = "Convert a private key to an address.")]
    Address {
        #[clap(
            help = "If provided, the address will be derived from the specified private key.",
            value_name = "PRIVATE_KEY",
            value_parser = foundry_common::clap_helpers::strip_0x_prefix
        )]
        private_key_override: Option<String>,
        #[clap(flatten)]
        wallet: Wallet,
    },
    #[clap(name = "sign", visible_alias = "s", about = "Sign payloads with your wallet.")]
    Sign {
        #[clap(subcommand)]
        command: Option<SignType>,
        #[clap(flatten)]
        wallet: Wallet,
    },
    #[clap(name = "verify", visible_alias = "v", about = "Verify the signature of a message.")]
    Verify {
        #[clap(help = "The original message.", value_name = "MESSAGE")]
        message: String,
        #[clap(help = "The signature to verify.", value_name = "SIGNATURE")]
        signature: String,
        #[clap(long, short, help = "The address of the message signer.", value_name = "ADDRESS")]
        address: String,
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
                        eyre::bail!("`{}` is not a directory.", path.display());
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
                        let sig = wallet.sign_message(message).await?;
                        println!("Signature: 0x{sig}");
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
                        println!("Signature: 0x{sig}");
                    }
                    None => {
                        println!("No subcommand provided. Please provide a subcommand for the type of data to sign.")
                    }
                }
            }
            WalletSubcommands::Verify { message, signature, address } => {
                let pubkey: Address = address.parse().wrap_err("Invalid address")?;
                let signature: Signature = signature.parse().wrap_err("Invalid signature")?;
                match signature.verify(message, pubkey) {
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
