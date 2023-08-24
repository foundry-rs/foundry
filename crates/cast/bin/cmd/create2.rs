use cast::SimpleCast;
use clap::Parser;
use ethers::{
    core::rand::thread_rng,
    types::{Address, Bytes, H256, U256},
    utils::{get_create2_address_from_hash, keccak256},
};
use eyre::{Result, WrapErr};
use rayon::prelude::*;
use regex::RegexSetBuilder;
use std::{str::FromStr, time::Instant};

/// CLI arguments for `cast create2`.
#[derive(Debug, Clone, Parser)]
pub struct Create2Args {
    /// Prefix for the contract address.
    #[clap(
        long,
        short,
        required_unless_present_any = &["ends_with", "matching"],
        value_name = "HEX"
    )]
    starts_with: Option<String>,

    /// Suffix for the contract address.
    #[clap(long, short, value_name = "HEX")]
    ends_with: Option<String>,

    /// Sequence that the address has to match.
    #[clap(long, short, value_name = "HEX")]
    matching: Option<String>,

    /// Case sensitive matching.
    #[clap(short, long)]
    case_sensitive: bool,

    /// Address of the contract deployer.
    #[clap(
        short,
        long,
        default_value = "0x4e59b44847b379578588920ca78fbf26c0b4956c",
        value_name = "ADDRESS"
    )]
    deployer: Address,

    /// Init code of the contract to be deployed.
    #[clap(short, long, value_name = "HEX", default_value = "")]
    init_code: String,

    /// Init code hash of the contract to be deployed.
    #[clap(alias = "ch", long, value_name = "HASH")]
    init_code_hash: Option<String>,
}

#[allow(dead_code)]
pub struct Create2Output {
    pub address: Address,
    pub salt: U256,
}

impl Create2Args {
    pub fn run(self) -> Result<Create2Output> {
        let Create2Args {
            starts_with,
            ends_with,
            matching,
            case_sensitive,
            deployer,
            init_code,
            init_code_hash,
        } = self;

        let mut regexs = vec![];

        if let Some(matches) = matching {
            if starts_with.is_some() || ends_with.is_some() {
                eyre::bail!("Either use --matching or --starts/ends-with");
            }

            let matches = matches.trim_start_matches("0x");

            if matches.len() != 40 {
                eyre::bail!("Please provide a 40 characters long sequence for matching");
            }

            hex::decode(matches.replace('X', "0")).wrap_err("invalid matching hex provided")?;
            // replacing X placeholders by . to match any character at these positions

            regexs.push(matches.replace('X', "."));
        }

        if let Some(prefix) = starts_with {
            regexs.push(format!(
                r"^{}",
                get_regex_hex_string(prefix).wrap_err("invalid prefix hex provided")?
            ));
        }
        if let Some(suffix) = ends_with {
            regexs.push(
                (get_regex_hex_string(suffix).wrap_err("invalid suffix hex provided")?).to_string(),
            );
        }

        debug_assert!(
            regexs.iter().map(|p| p.len() - 1).sum::<usize>() <= 40,
            "vanity patterns length exceeded. cannot be more than 40 characters",
        );

        let regex = RegexSetBuilder::new(regexs).case_insensitive(!case_sensitive).build()?;

        let init_code_hash = if let Some(init_code_hash) = init_code_hash {
            let mut a: [u8; 32] = [0; 32];
            let init_code_hash = init_code_hash.strip_prefix("0x").unwrap_or(&init_code_hash);
            assert!(init_code_hash.len() == 64, "init code hash should be 32 bytes long"); // 32 bytes * 2
            a.copy_from_slice(&hex::decode(init_code_hash)?[..32]);
            a
        } else {
            let init_code = init_code.strip_prefix("0x").unwrap_or(&init_code).as_bytes();
            keccak256(hex::decode(init_code)?)
        };

        println!("Starting to generate deterministic contract address...");
        let timer = Instant::now();
        let (salt, addr) = std::iter::repeat(())
            .par_bridge()
            .map(|_| {
                let salt = H256::random_using(&mut thread_rng());
                let salt = Bytes::from(salt.to_fixed_bytes());

                let addr = SimpleCast::to_checksum_address(&get_create2_address_from_hash(
                    deployer,
                    salt.clone(),
                    init_code_hash,
                ));

                (salt, addr)
            })
            .find_any(move |(_, addr)| {
                let addr = addr.to_string();
                let addr = addr.strip_prefix("0x").unwrap();
                regex.matches(addr).into_iter().count() == regex.patterns().len()
            })
            .unwrap();

        let salt = U256::from(salt.to_vec().as_slice());
        let address = Address::from_str(&addr).unwrap();

        println!(
            "Successfully found contract address in {} seconds.\nAddress: {}\nSalt: {}",
            timer.elapsed().as_secs(),
            addr,
            salt
        );

        Ok(Create2Output { address, salt })
    }
}

fn get_regex_hex_string(s: String) -> Result<String> {
    let s = s.strip_prefix("0x").unwrap_or(&s);
    let pad_width = s.len() + s.len() % 2;
    hex::decode(format!("{s:0>pad_width$}"))?;
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use ethers::{abi::AbiEncode, utils::get_create2_address};

    use super::*;

    const DEPLOYER: &str = "0x4e59b44847b379578588920ca78fbf26c0b4956c";

    #[test]
    fn basic_create2() {
        // even hex chars
        let args = Create2Args::parse_from(["foundry-cli", "--starts-with", "aa"]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        let address = format!("{address:x}");

        assert!(address.starts_with("aa"));

        // odd hex chars
        let args = Create2Args::parse_from(["foundry-cli", "--starts-with", "aaa"]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        let address = format!("{address:x}");

        assert!(address.starts_with("aaa"));

        // even hex chars with 0x prefix
        let args = Create2Args::parse_from(["foundry-cli", "--starts-with", "0xaa"]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        let address = format!("{address:x}");

        assert!(address.starts_with("aa"));

        // odd hex chars with 0x prefix
        let args = Create2Args::parse_from(["foundry-cli", "--starts-with", "0xaaa"]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        let address = format!("{address:x}");

        assert!(address.starts_with("aaa"));

        // TODO: add check fails on wrong chars
        // let args = Create2Args::parse_from(["foundry-cli", "--starts-with", "0xtest"]);
        // let create2_out = args.run();
        // assert_eq!(create2_out, Err("invalid prefix hex provided"));
    }

    #[test]
    fn matches_pattern() {
        let args = Create2Args::parse_from([
            "foundry-cli",
            "--matching",
            "0xbbXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
        ]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        let address = format!("{address:x}");

        assert!(address.starts_with("bb"));
    }

    #[test]
    fn create2_init_code() {
        let init_code = "00";
        let args = Create2Args::parse_from([
            "foundry-cli",
            "--starts-with",
            "cc",
            "--init-code",
            init_code,
        ]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        let address_str = format!("{address:x}");
        let salt = create2_out.salt;
        let deployer = Address::from_str(DEPLOYER).unwrap();

        assert!(address_str.starts_with("cc"));
        assert_eq!(address, verify_create2(deployer, salt, hex::decode(init_code).unwrap()));
    }

    #[test]
    fn create2_init_code_hash() {
        let init_code_hash = "bc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a";
        let args = Create2Args::parse_from([
            "foundry-cli",
            "--starts-with",
            "dd",
            "--init-code-hash",
            init_code_hash,
        ]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        let address_str = format!("{address:x}");
        let salt = create2_out.salt;
        let deployer = Address::from_str(DEPLOYER).unwrap();

        assert!(address_str.starts_with("dd"));
        assert_eq!(
            address,
            verify_create2_hash(deployer, salt, hex::decode(init_code_hash).unwrap())
        );
    }

    #[test]
    fn verify_helpers() {
        // https://eips.ethereum.org/EIPS/eip-1014
        let eip_address = Address::from_str("0x4D1A2e2bB4F88F0250f26Ffff098B0b30B26BF38").unwrap();

        let deployer = Address::from_str("0x0000000000000000000000000000000000000000").unwrap();
        let salt =
            U256::from_str("0x0000000000000000000000000000000000000000000000000000000000000000")
                .unwrap();
        let init_code = hex::decode("00").unwrap();
        let address = verify_create2(deployer, salt, init_code);

        assert_eq!(address, eip_address);

        let init_code_hash =
            hex::decode("bc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a")
                .unwrap();
        let address = verify_create2_hash(deployer, salt, init_code_hash);

        assert_eq!(address, eip_address);
    }

    fn verify_create2(deployer: Address, salt: U256, init_code: Vec<u8>) -> Address {
        // let init_code_hash = keccak256(init_code);
        get_create2_address(deployer, salt.encode(), init_code)
    }

    fn verify_create2_hash(deployer: Address, salt: U256, init_code_hash: Vec<u8>) -> Address {
        get_create2_address_from_hash(deployer, salt.encode(), init_code_hash)
    }
}
