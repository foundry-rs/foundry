//! cast wallet subcommand

use crate::opts::{EthereumOpts, Wallet, WalletType};
use cast::SimpleCast;
use clap::Parser;
use ethers::{
    core::rand::thread_rng,
    signers::{LocalWallet, Signer},
    types::{Address, Chain, Signature},
    utils::get_contract_address,
};
use rayon::prelude::*;
use regex::RegexSet;
use std::{str::FromStr, time::Instant};

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
            conflicts_with = "unsafe-password",
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
    Vanity {
        #[clap(
            long,
            help = "Prefix for the vanity address.",
            required_unless_present = "ends-with",
            value_name = "HEX"
        )]
        starts_with: Option<String>,
        #[clap(long, help = "Suffix for the vanity address.", value_name = "HEX")]
        ends_with: Option<String>,
        #[clap(
            long,
            help = "Generate a vanity contract address created by the generated keypair with the specified nonce.",
            value_name = "NONCE"
        )]
        nonce: Option<u64>, /* 2^64-1 is max possible nonce per https://eips.ethereum.org/EIPS/eip-2681 */
    },
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
                    let address = SimpleCast::checksum_address(&key.address())?;
                    let filepath = path.join(uuid);

                    println!(
                        r#"Created new encrypted keystore file: `{}`\nPublic Address of the key: {}"#,
                        filepath.display(),
                        address
                    );
                } else {
                    let wallet = LocalWallet::new(&mut rng);
                    println!(
                        "Successfully created new keypair.\nAddress: {}\nPrivate Key: {}",
                        SimpleCast::checksum_address(&wallet.address())?,
                        hex::encode(wallet.signer().to_bytes()),
                    );
                }
            }
            WalletSubcommands::Vanity { starts_with, ends_with, nonce } => {
                let mut regexs = vec![];
                if let Some(prefix) = starts_with {
                    let pad_width = prefix.len() + prefix.len() % 2;
                    hex::decode(format!("{:0>width$}", prefix, width = pad_width))
                        .expect("invalid prefix hex provided");
                    regexs.push(format!(r"^{}", prefix));
                }
                if let Some(suffix) = ends_with {
                    let pad_width = suffix.len() + suffix.len() % 2;
                    hex::decode(format!("{:0>width$}", suffix, width = pad_width))
                        .expect("invalid suffix hex provided");
                    regexs.push(format!(r"{}$", suffix));
                }

                assert!(
                    regexs.iter().map(|p| p.len() - 1).sum::<usize>() <= 40,
                    "vanity patterns length exceeded. cannot be more than 40 characters",
                );

                let regex = RegexSet::new(regexs)?;
                let match_contract = nonce.is_some();

                println!("Starting to generate vanity address...");
                let timer = Instant::now();
                let wallet = std::iter::repeat_with(move || LocalWallet::new(&mut thread_rng()))
                    .par_bridge()
                    .find_any(|wallet| {
                        let addr = if match_contract {
                            // looking for contract address created by wallet with CREATE + nonce
                            let contract_addr =
                                get_contract_address(wallet.address(), nonce.unwrap());
                            hex::encode(contract_addr.to_fixed_bytes())
                        } else {
                            // looking for wallet address
                            hex::encode(wallet.address().to_fixed_bytes())
                        };
                        regex.matches(&addr).into_iter().count() == regex.patterns().len()
                    })
                    .expect("failed to generate vanity wallet");

                println!(
                    "Successfully found vanity address in {} seconds.{}{}\nAddress: {}\nPrivate Key: 0x{}",
                    timer.elapsed().as_secs(),
                    if match_contract {"\nContract address: "} else {""},
                    if match_contract {SimpleCast::checksum_address(&get_contract_address(wallet.address(), nonce.unwrap()))?} else {"".to_string()},
                    SimpleCast::checksum_address(&wallet.address())?,
                    hex::encode(wallet.signer().to_bytes()),
                );
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
                };
                println!("Address: {}", SimpleCast::checksum_address(&addr)?);
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
                };
                println!("Signature: 0x{sig}");
            }
            WalletSubcommands::Verify { message, signature, address } => {
                let pubkey = Address::from_str(&address).expect("invalid pubkey provided");
                let signature = Signature::from_str(&signature)?;
                match signature.verify(message, pubkey) {
                    Ok(_) => {
                        println!("Validation success. Address {address} signed this message.")
                    }
                    Err(_) => println!(
                        "Validation failed. Address {} did not sign this message.",
                        address
                    ),
                }
            }
        };

        Ok(())
    }
}
