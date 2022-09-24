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
use rayon::prelude::*;
use regex::RegexSetBuilder;
use std::time::Instant;

#[derive(Debug, Clone, Parser)]
pub struct Create2Args {
    #[clap(
        long,
        short,
        help = "Prefix for the contract address.",
        required_unless_present_any = &["ends-with", "matching"],
        value_name = "HEX"
    )]
    starts_with: Option<String>,
    #[clap(long, short, help = "Suffix for the contract address.", value_name = "HEX")]
    ends_with: Option<String>,
    #[clap(long, short, help = "Sequence that the address has to match", value_name = "HEX")]
    matching: Option<String>,
    #[clap(short, long)]
    case_sensitive: bool,
    #[clap(
        short,
        long,
        help = "Address of the contract deployer.",
        default_value = "0x4e59b44847b379578588920ca78fbf26c0b4956c",
        value_name = "ADDRESS"
    )]
    deployer: Address,
    #[clap(
        short,
        long,
        help = "Init code of the contract to be deployed.",
        value_name = "HEX",
        default_value = ""
    )]
    init_code: String,
}

impl Cmd for Create2Args {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        Create2Args::generate_address(self)
    }
}

impl Create2Args {
    fn generate_address(self) -> Result<()> {
        let Create2Args { starts_with, ends_with, matching, case_sensitive, deployer, init_code } =
            self;

        let mut regexs = vec![];

        if let Some(mut matches) = matching {
            if starts_with.is_some() || ends_with.is_some() {
                eyre::bail!("Either use --matching or --starts/ends-with");
            }

            if matches.len() != 40 {
                if !(matches.len() == 42 && matches.starts_with("0x")) {
                    eyre::bail!("Please provide a 40 characters long sequence for matching");
                }
                matches = matches.strip_prefix("0x").unwrap().to_string();
            }

            hex::decode(&matches.replace('X', "0")).wrap_err("invalid matching hex provided")?;
            regexs.push(matches.replace('X', "."));
        }

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

        let regex = RegexSetBuilder::new(regexs).case_insensitive(!case_sensitive).build()?;

        let init_code = Bytes::from(init_code.into_bytes());

        println!("Starting to generate deterministic contract address...");
        let timer = Instant::now();
        let (salt, addr) = std::iter::repeat(())
            .par_bridge()
            .map(|_| {
                let salt = H256::random_using(&mut thread_rng());
                let salt = Bytes::from(salt.to_fixed_bytes());

                let init_code = init_code.clone();

                let addr = SimpleCast::to_checksum_address(&get_create2_address_from_hash(
                    deployer,
                    salt.clone(),
                    init_code,
                ));

                (salt, addr)
            })
            .find_any(move |(_, addr)| {
                let addr = addr.to_string();
                let addr = addr.strip_prefix("0x").unwrap();
                regex.matches(addr).into_iter().count() == regex.patterns().len()
            })
            .unwrap();

        println!(
            "Successfully found contract address in {} seconds.\nAddress: {}\nSalt: {}",
            timer.elapsed().as_secs(),
            addr,
            U256::from(salt.to_vec().as_slice())
        );

        Ok(())
    }
}
