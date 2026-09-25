use alloy_dyn_abi::JsonAbiExt;
use alloy_primitives::{Address, B256, U256, hex, hex::FromHex, keccak256};
use clap::{Args, Parser, Subcommand};
use eyre::{Result, WrapErr};
use foundry_cli::{
    json::print_scalar,
    opts::BuildOpts,
    utils::{LoadConfig, find_contract_artifacts, parse_constructor_args},
};
use foundry_common::{compile, shell};
use foundry_compilers::{info::ContractInfo, utils::canonicalize};
use rand::{RngCore, SeedableRng, rngs::StdRng};
use regex::RegexSetBuilder;
use std::time::Instant;

// https://etherscan.io/address/0x4e59b44847b379578588920ca78fbf26c0b4956c#code
const DEPLOYER: &str = "0x4e59b44847b379578588920ca78fbf26c0b4956c";

#[derive(Clone, Debug, Subcommand)]
enum Create2Subcommand {
    /// Compute a contract's CREATE2 init code hash.
    #[command(visible_alias = "initcodehash")]
    InitCodeHash(InitCodeHashArgs),
}

foundry_config::impl_figment_convert!(InitCodeHashArgs, build);

#[derive(Clone, Debug, Args)]
struct InitCodeHashArgs {
    /// The contract identifier in the form `<path>:<contractname>`.
    contract: ContractInfo,

    /// The constructor arguments.
    #[arg(value_name = "ARGS", allow_negative_numbers = true)]
    constructor_args: Vec<String>,

    #[command(flatten)]
    build: BuildOpts,
}

impl InitCodeHashArgs {
    fn run(&self) -> Result<()> {
        let config = self.load_config()?;
        let project = config.project()?;
        let target_path = if let Some(path) = &self.contract.path {
            canonicalize(project.root().join(path))?
        } else {
            project.find_contract_path(&self.contract.name)?
        };

        let output = compile::compile_target(&target_path, &project, true)?;
        let (abi, bin, _) = find_contract_artifacts(output, &target_path, &self.contract.name)?;
        let Some(bytecode) = bin.object.into_bytes() else {
            eyre::bail!("contract contains unlinked libraries");
        };
        if bytecode.is_empty() {
            eyre::bail!("no bytecode found in bin object for {}", self.contract.name);
        }

        let mut init_code = bytecode.to_vec();
        if let Some(constructor) = &abi.constructor {
            let params = parse_constructor_args(constructor, &self.constructor_args)?;
            init_code.extend(constructor.abi_encode_input(&params)?);
        } else if !self.constructor_args.is_empty() {
            eyre::bail!("contract does not have a constructor");
        }

        print_scalar(keccak256(init_code))?;
        Ok(())
    }
}

/// CLI arguments for `cast create2`.
#[derive(Clone, Debug, Parser)]
#[command(subcommand_negates_reqs = true, args_conflicts_with_subcommands = true)]
pub struct Create2Args {
    #[command(subcommand)]
    command: Option<Create2Subcommand>,

    /// Prefix for the contract address.
    #[arg(
        long,
        short,
        required_unless_present_any = &["ends_with", "matching", "salt"],
        value_name = "HEX"
    )]
    starts_with: Option<String>,

    /// Suffix for the contract address.
    #[arg(long, short, value_name = "HEX")]
    ends_with: Option<String>,

    /// Sequence that the address has to match.
    #[arg(long, short, value_name = "HEX")]
    matching: Option<String>,

    /// Case sensitive matching.
    #[arg(short, long)]
    case_sensitive: bool,

    /// Address of the contract deployer.
    #[arg(
        short,
        long,
        default_value = DEPLOYER,
        value_name = "ADDRESS"
    )]
    deployer: Address,

    /// Salt to be used for the contract deployment. This option separate from the default salt
    /// mining with filters.
    #[arg(
        long,
        conflicts_with_all = [
            "starts_with",
            "ends_with",
            "matching",
            "case_sensitive",
            "caller",
            "seed",
            "no_random"
        ],
        value_name = "HEX"
    )]
    salt: Option<String>,

    /// Init code of the contract to be deployed.
    #[arg(short, long, value_name = "HEX")]
    init_code: Option<String>,

    /// Init code hash of the contract to be deployed.
    #[arg(alias = "ch", long, value_name = "HASH", required_unless_present = "init_code")]
    init_code_hash: Option<String>,

    /// Number of threads to use. Specifying 0 defaults to the number of logical cores.
    #[arg(global = true, long, short = 'j', visible_alias = "jobs")]
    threads: Option<usize>,

    /// Address of the caller. Used for the first 20 bytes of the salt.
    #[arg(long, value_name = "ADDRESS")]
    caller: Option<Address>,

    /// The random number generator's seed, used to initialize the salt.
    #[arg(long, value_name = "HEX")]
    seed: Option<B256>,

    /// Don't initialize the salt with a random value, and instead use the default value of 0.
    #[arg(long, conflicts_with = "seed")]
    no_random: bool,
}

impl Create2Args {
    pub fn execute(self) -> Result<()> {
        if let Some(Create2Subcommand::InitCodeHash(args)) = &self.command {
            return args.run();
        }
        self.run().map(drop)
    }

    /// Mines (or derives) the salt and returns the resulting address and salt.
    fn run(self) -> Result<(Address, B256)> {
        let Self {
            command: _,
            starts_with,
            ends_with,
            matching,
            case_sensitive,
            deployer,
            salt,
            init_code,
            init_code_hash,
            threads,
            caller,
            seed,
            no_random,
        } = self;

        let init_code_hash = match (init_code_hash, init_code) {
            (Some(init_code_hash), _) => B256::from_hex(init_code_hash)?,
            // Clap requires one of the two.
            (None, init_code) => keccak256(hex::decode(init_code.unwrap_or_default())?),
        };

        if let Some(salt) = salt {
            let salt = B256::from_hex(salt)?;
            let address = deployer.create2(salt, init_code_hash);
            sh_println!("{address}\t{salt}")?;
            return Ok((address, salt));
        }

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
            regexs.push(format!(
                r"{}$",
                get_regex_hex_string(suffix).wrap_err("invalid suffix hex provided")?
            ))
        }

        debug_assert!(
            regexs.iter().map(|p| p.len() - 1).sum::<usize>() <= 40,
            "vanity patterns length exceeded. cannot be more than 40 characters",
        );

        let regex = RegexSetBuilder::new(regexs).case_insensitive(!case_sensitive).build()?;

        let mut n_threads = match threads {
            Some(n) if n != 0 => n,
            _ => std::thread::available_parallelism().map_or(1, |n| n.get()),
        };
        if cfg!(test) {
            n_threads = n_threads.min(2);
        }

        let mut salt = B256::ZERO;
        let remaining = if let Some(caller_address) = caller {
            salt[..20].copy_from_slice(&caller_address.into_array());
            &mut salt[20..]
        } else {
            &mut salt[..]
        };

        if !no_random {
            let mut rng = match seed {
                Some(seed) => StdRng::from_seed(seed.0),
                None => StdRng::from_os_rng(),
            };
            rng.fill_bytes(remaining);
        }

        sh_status!("Configuration:")?;
        sh_status!("Init code hash: {init_code_hash}")?;
        sh_status!("Regex patterns: {:?}\n", regex.patterns())?;
        sh_status!(
            "Starting to generate deterministic contract address with {n_threads} threads..."
        )?;
        let timer = Instant::now();
        let regex_len = regex.patterns().len();
        let mut checksum_buf = [0u8; 42];
        let mut hex_buf = [0u8; 40];
        let (address, salt) = super::miner::mine_salt(salt, n_threads, move |salt| {
            #[expect(clippy::needless_borrows_for_generic_args)]
            let addr = deployer.create2(&salt, &init_code_hash);
            // Use checksum format only when case_sensitive is enabled — it requires an extra
            // keccak256 call, so we fall back to plain hex when case sensitivity is off.
            let s = if case_sensitive {
                let _ = addr.to_checksum_raw(&mut checksum_buf, None);
                // SAFETY: stripping 2 ASCII bytes ("0x") off of an already valid UTF-8 string.
                unsafe { std::str::from_utf8_unchecked(checksum_buf.get_unchecked(2..)) }
            } else {
                // SAFETY: hex::encode_to_slice always produces valid UTF-8 (hex digits).
                let _ = hex::encode_to_slice(addr.as_slice(), &mut hex_buf);
                unsafe { std::str::from_utf8_unchecked(&hex_buf) }
            };
            (regex.matches(s).into_iter().count() == regex_len).then_some((addr, salt))
        })
        .ok_or_else(|| eyre::eyre!("create2 salt mining failed: all threads panicked"))?;
        sh_status!("Successfully found contract address in {:?}", timer.elapsed())?;
        sh_status!("Address: {address}")?;
        sh_status!("Salt: {salt} ({})", U256::from_be_bytes(salt.0))?;
        // The machine-readable stdout record duplicates the prose above when stdout is an
        // interactive terminal.
        if !shell::is_out_tty() {
            sh_println!("{address}\t{salt}")?;
        }

        Ok((address, salt))
    }
}

fn get_regex_hex_string(s: String) -> Result<String> {
    let s = s.strip_prefix("0x").unwrap_or(&s);
    let pad_width = s.len() + s.len() % 2;
    hex::decode(format!("{s:0<pad_width$}"))?;
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{address, b256};
    use std::str::FromStr;

    const ZERO_HASH: &str =
        "--init-code-hash=0x0000000000000000000000000000000000000000000000000000000000000000";

    fn run(args: &[&str]) -> Result<(Address, B256)> {
        Create2Args::parse_from(["foundry-cli"].iter().chain(args)).run()
    }

    #[test]
    fn basic_create2() {
        for (flag, pattern) in [
            ("--starts-with", "aa"),
            ("--ends-with", "bb"),
            ("--starts-with", "aaa"),
            ("--ends-with", "bbb"),
            ("--starts-with", "0xaa"),
            ("--starts-with", "0xaaa"),
        ] {
            let (address, _) = run(&[ZERO_HASH, flag, pattern]).unwrap();
            let address = format!("{address:x}");
            let pattern = pattern.trim_start_matches("0x");
            assert!(
                if flag == "--starts-with" {
                    address.starts_with(pattern)
                } else {
                    address.ends_with(pattern)
                },
                "{flag} {pattern}: {address}"
            );
        }

        // Non-hex and misplaced prefixes are rejected.
        assert!(run(&[ZERO_HASH, "--starts-with", "0xerr"]).is_err());
        assert!(run(&[ZERO_HASH, "--starts-with", "x00"]).is_err());
    }

    #[test]
    fn matches_pattern() {
        let (address, _) =
            run(&[ZERO_HASH, "--matching=0xbbXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"]).unwrap();
        assert!(format!("{address:x}").starts_with("bb"));
    }

    #[test]
    fn create2_salt() {
        let (address, _) = run(&[
            "--deployer=0x8ba1f109551bD432803012645Ac136ddd64DBA72",
            "--salt=0x7c5ea36004851c764c44143b1dcb59679b11c9a68e5f41497f6cf3d480715331",
            "--init-code=0x6394198df16000526103ff60206004601c335afa6040516060f3",
        ])
        .unwrap();
        assert_eq!(address, address!("0x533AE9D683B10C02EBDB05471642F85230071FC3"));
    }

    #[test]
    fn create2_init_code() {
        let init_code = "00";
        let (address, salt) = run(&["--starts-with=cc", "--init-code", init_code]).unwrap();
        assert!(format!("{address:x}").starts_with("cc"));
        let deployer = Address::from_str(DEPLOYER).unwrap();
        assert_eq!(address, deployer.create2_from_code(salt, hex::decode(init_code).unwrap()));
    }

    #[test]
    fn create2_init_code_hash() {
        let init_code_hash = "bc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a";
        let (address, salt) =
            run(&["--starts-with=dd", "--init-code-hash", init_code_hash]).unwrap();
        assert!(format!("{address:x}").starts_with("dd"));
        let deployer = Address::from_str(DEPLOYER).unwrap();
        assert_eq!(address, deployer.create2(salt, B256::from_str(init_code_hash).unwrap()));
    }

    #[test]
    fn create2_caller() {
        let (address, salt) = run(&[
            "--starts-with=dd",
            "--init-code-hash=bc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a",
            "--caller=0x66f9664f97F2b50F62D13eA064982f936dE76657",
        ])
        .unwrap();
        assert!(format!("{address:x}").starts_with("dd"));
        assert!(format!("{salt:x}").starts_with("66f9664f97f2b50f62d13ea064982f936de76657"));
    }

    #[test]
    fn deterministic_seed() {
        let (address, salt) = run(&[
            "--starts-with=0x00",
            "--init-code-hash=0x479d7e8f31234e208d704ba1a123c76385cea8a6981fd675b784fbd9cffb918d",
            "--seed=0x479d7e8f31234e208d704ba1a123c76385cea8a6981fd675b784fbd9cffb918d",
            "-j1",
        ])
        .unwrap();
        assert_eq!(address, address!("0x00614b3D65ac4a09A376a264fE1aE5E5E12A6C43"));
        assert_eq!(
            salt,
            b256!("0x322113f523203e2c0eb00bbc8e69208b0eb0c8dad0eaac7b01d64ff016edb40d")
        );
    }

    #[test]
    fn deterministic_output() {
        let (address, salt) = run(&[
            "--starts-with=0x00",
            "--init-code-hash=0x479d7e8f31234e208d704ba1a123c76385cea8a6981fd675b784fbd9cffb918d",
            "--no-random",
            "-j1",
        ])
        .unwrap();
        assert_eq!(address, address!("0x00bF495b8b42fdFeb91c8bCEB42CA4eE7186AEd2"));
        assert_eq!(
            salt,
            b256!("0x000000000000000000000000000000000000000000000000df00000000000000")
        );
    }
}
