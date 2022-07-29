//! cast create2 subcommand

use crate::cmd::Cmd;
use cast::SimpleCast;
use clap::Parser;
use ethers::{
    core::rand::thread_rng,
    types::{Address, Bytes, H256, U256},
    utils::get_create2_address_from_hash,
};
use eyre::{Result, WrapErr};
use futures::future::BoxFuture;
use rayon::prelude::*;
use regex::RegexSet;
use std::time::Instant;

#[derive(Debug, Clone, Parser)]
pub struct Create2Args {
    #[clap(
        long,
        help = "Prefix for the contract address.",
        required_unless_present_any = &["ends-with", "matching"],
        value_name = "HEX"
    )]
    starts_with: Option<String>,
    #[clap(long, help = "Suffix for the contract address.", value_name = "HEX")]
    ends_with: Option<String>,
    #[clap(long, help = "Sequence that the address has to match", value_name = "HEX")]
    matching: Option<String>,
    #[clap(short, long)]
    case_sentitive: bool,
    #[clap(short, long, help = "Address of the contract deployer.")]
    deployer: Address,
    #[clap(short, long, help = "Init code of the contract to be deployed.", value_name = "HEX")]
    init_code: String,
}

impl Cmd for Create2Args {
    type Output = BoxFuture<'static, Result<()>>;

    fn run(self) -> Result<Self::Output> {
        let Create2Args { starts_with, ends_with, matching, case_sentitive, deployer, init_code } =
            self;
        Ok(Box::pin(Self::generate_address(
            starts_with,
            ends_with,
            matching,
            case_sentitive,
            deployer,
            init_code,
        )))
    }
}

impl Create2Args {
    async fn generate_address(
        starts_with: Option<String>,
        ends_with: Option<String>,
        matching: Option<String>,
        case_sentitive: bool,
        deployer: Address,
        init_code: String,
    ) -> Result<()> {
        if matching.is_some() {
            let matches = matching.unwrap();
            if starts_with.is_some() || ends_with.is_some() {
                eyre::bail!("Either use --matching or --starts/ends-with");
            }

            if matches.len() != 40 {
                eyre::bail!("Please provide a 40 bytes long sequence for matching");
            }

            hex::decode(matches).wrap_err("invalid matching hex provided")?; // TODO: build regex
        }

        let mut regexs = vec![];
        if let Some(prefix) = starts_with {
            let pad_width = prefix.len() + prefix.len() % 2;
            hex::decode(format!("{:0>width$}", prefix, width = pad_width))
                .wrap_err("invalid prefix hex provided")?;
            regexs.push(format!(r"^{}", prefix));
        }
        if let Some(suffix) = ends_with {
            let pad_width = suffix.len() + suffix.len() % 2;
            hex::decode(format!("{:0>width$}", suffix, width = pad_width))
                .wrap_err("invalid suffix hex provided")?;
            regexs.push(format!(r"{}$", suffix));
        }

        assert!(
            regexs.iter().map(|p| p.len() - 1).sum::<usize>() <= 40,
            "vanity patterns length exceeded. cannot be more than 40 characters",
        );

        let regex = RegexSet::new(regexs)?;

        let init_code = Bytes::from(init_code.into_bytes());

        println!("Starting to generate deterministic contract address...");
        let timer = Instant::now();
        let salt = std::iter::repeat_with(move || H256::random_using(&mut thread_rng()))
            .par_bridge()
            .find_any(|salt| {
                let salt = Bytes::from(salt.to_fixed_bytes());
                let init_code = init_code.clone();

                let addr = get_create2_address_from_hash(deployer, salt, init_code);
                let addr = hex::encode(addr);

                regex.matches(&addr).into_iter().count() == regex.patterns().len()
            })
            .expect("failed to generate vanity wallet");

        println!(
            "Successfully found contract address in {} seconds.\nAddress: {}\nSalt: {}",
            timer.elapsed().as_secs(),
            SimpleCast::checksum_address(&get_create2_address_from_hash(
                deployer,
                Bytes::from(salt.to_fixed_bytes()),
                init_code
            ))?,
            U256::from(salt.to_fixed_bytes())
        );

        Ok(())
    }
}
