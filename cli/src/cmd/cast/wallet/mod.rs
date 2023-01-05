//! cast wallet subcommand

pub mod vanity;

use crate::{
    cmd::{cast::wallet::vanity::VanityArgs, Cmd},
    opts::{EthereumOpts, Wallet, WalletType},
};
use cast::SimpleCast;
use clap::Parser;
use ethers::{
    core::rand::thread_rng,
    signers::{LocalWallet, Signer},
    types::{Address, Chain, Signature},
};
use eyre::Context;

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
    #[clap(name = "sign", visible_alias = "s", about = "Sign a message.")]
    Sign {
        #[clap(help = "message to sign", value_name = "MESSAGE")]
        message: String,
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
                        eprintln!("`{}` is not a directory.", path.display());
                        std::process::exit(1)
                    }

                    let password = if let Some(password) = unsafe_password {
                        password
                    } else {
                        // if no --unsafe-password was provided read via stdin
                        rpassword::prompt_password("Enter secret: ")?
                    };

                    let (key, uuid) = LocalWallet::new_keystore(&path, &mut rng, password, None)?;
                    let address = SimpleCast::to_checksum_address(&key.address());
                    let filepath = path.join(uuid);

                    println!(
                        r#"Created new encrypted keystore file: `{}`\nPublic Address of the key: {}"#,
                        filepath.display(),
                        address
                    );
                } else {
                    let wallet = LocalWallet::new(&mut rng);
                    println!(
                        "Successfully created new keypair.\nAddress: {}\nPrivate Key: 0x{}",
                        SimpleCast::to_checksum_address(&wallet.address()),
                        hex::encode(wallet.signer().to_bytes()),
                    );
                }
            }
            WalletSubcommands::Vanity(cmd) => {
                cmd.run()?;
            }
            WalletSubcommands::Address { wallet, private_key_override } => {
                let wallet = EthereumOpts {
                    wallet: private_key_override
                        .map(|pk| Wallet { private_key: Some(pk), ..Default::default() })
                        .unwrap_or(wallet),
                    rpc_url: Some("http://localhost:8545".to_string()),
                    chain: Some(Chain::Mainnet.into()),
                    ..Default::default()
                }
                .signer(0u64.into())
                .await?
                .unwrap();

                let addr = match wallet {
                    WalletType::Ledger(signer) => signer.address(),
                    WalletType::Local(signer) => signer.address(),
                    WalletType::Trezor(signer) => signer.address(),
                    WalletType::Aws(signer) => signer.address(),
                };
                println!("{}", SimpleCast::to_checksum_address(&addr));
            }
            WalletSubcommands::Sign { message, wallet } => {
                let wallet = EthereumOpts {
                    wallet,
                    rpc_url: Some("http://localhost:8545".to_string()),
                    chain: Some(Chain::Mainnet.into()),
                    ..Default::default()
                }
                .signer(0u64.into())
                .await?
                .unwrap();

                let sig = match wallet {
                    WalletType::Ledger(wallet) => wallet.signer().sign_message(&message).await?,
                    WalletType::Local(wallet) => wallet.signer().sign_message(&message).await?,
                    WalletType::Trezor(wallet) => wallet.signer().sign_message(&message).await?,
                    WalletType::Aws(wallet) => wallet.signer().sign_message(&message).await?,
                };
                println!("Signature: 0x{sig}");
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
