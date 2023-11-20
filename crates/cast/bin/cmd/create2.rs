use alloy_primitives::{keccak256, Address, B256, U256};
use clap::Parser;
use eyre::{Result, WrapErr};
use regex::RegexSetBuilder;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

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
    #[clap(short, long, value_name = "HEX")]
    init_code: Option<String>,

    /// Init code hash of the contract to be deployed.
    #[clap(alias = "ch", long, value_name = "HASH", required_unless_present = "init_code")]
    init_code_hash: Option<String>,

    /// Number of threads to use. Defaults to and caps at the number of logical cores.
    #[clap(short, long)]
    jobs: Option<usize>,

    /// Address of the caller. Used for the first 20 bytes of the salt.
    #[clap(long, value_name = "ADDRESS")]
    caller: Option<Address>,
}

#[allow(dead_code)]
pub struct Create2Output {
    pub address: Address,
    pub salt: B256,
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
            jobs,
            caller,
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
            regexs.push(format!(
                r"{}$",
                get_regex_hex_string(suffix).wrap_err("invalid prefix hex provided")?
            ))
        }

        debug_assert!(
            regexs.iter().map(|p| p.len() - 1).sum::<usize>() <= 40,
            "vanity patterns length exceeded. cannot be more than 40 characters",
        );

        let regex = RegexSetBuilder::new(regexs).case_insensitive(!case_sensitive).build()?;

        let init_code_hash = if let Some(init_code_hash) = init_code_hash {
            let mut hash: [u8; 32] = [0; 32];
            hex::decode_to_slice(init_code_hash, &mut hash)?;
            hash.into()
        } else if let Some(init_code) = init_code {
            keccak256(hex::decode(init_code)?)
        } else {
            unreachable!();
        };

        let mut n_threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        if let Some(jobs) = jobs {
            n_threads = n_threads.min(jobs);
        }
        if cfg!(test) {
            n_threads = n_threads.min(2);
        }

        let top_bytes = match caller {
            None => B256::ZERO,
            Some(caller_address) => {
                let mut slice = B256::ZERO;
                slice[..20].copy_from_slice(&caller_address.into_array());
                slice
            }
        };

        println!("Starting to generate deterministic contract address...");
        let mut handles = Vec::with_capacity(n_threads);
        let found = Arc::new(AtomicBool::new(false));
        let timer = Instant::now();

        // Loops through all possible salts in parallel until a result is found.
        // Each thread iterates over `(i..).step_by(n_threads)`.
        for i in 0..n_threads {
            // Create local copies for the thread.
            let increment = n_threads;
            let regex = regex.clone();
            let regex_len = regex.patterns().len();
            let found = Arc::clone(&found);
            handles.push(std::thread::spawn(move || {
                // Read the first bytes of the salt as a usize to be able to increment it.
                struct B256Aligned(B256, [usize; 0]);
                let mut salt = B256Aligned(top_bytes, []);
                // SAFETY: B256 is aligned to `usize`.
                let salt_word = unsafe {
                    let ptr: *mut u8 = &mut salt.0[0];
                    // Offset to preserve caller address at the beginning of the salt.
                    // Must offset by 24 and not 20 to align u8s and usize.
                    &mut *ptr.add(24).cast::<usize>()
                };

                // Important: set the salt to the start value, otherwise all threads loop over the
                // same values.
                *salt_word = i;

                let mut checksum = [0; 42];
                loop {
                    // Stop if a result was found in another thread.
                    if found.load(Ordering::Relaxed) {
                        break None;
                    }

                    // Calculate the `CREATE2` address.
                    #[allow(clippy::needless_borrows_for_generic_args)]
                    let addr = deployer.create2(&salt.0, init_code_hash);

                    // Check if the the regex matches the calculated address' checksum.
                    let _ = addr.to_checksum_raw(&mut checksum, None);
                    // SAFETY: stripping 2 ASCII bytes ("0x") off of an already valid UTF-8 string
                    // is safe.
                    let s = unsafe { std::str::from_utf8_unchecked(checksum.get_unchecked(2..)) };
                    if regex.matches(s).into_iter().count() == regex_len {
                        // Notify other threads that we found a result.
                        found.store(true, Ordering::Relaxed);
                        break Some((salt.0, addr));
                    }

                    // Increment the salt for the next iteration.
                    *salt_word += increment;
                }
            }));
        }

        let results = handles.into_iter().filter_map(|h| h.join().unwrap()).collect::<Vec<_>>();
        println!("Successfully found contract address(es) in {:?}", timer.elapsed());
        for (i, (salt, address)) in results.iter().enumerate() {
            if i > 0 {
                println!("---");
            }
            println!("Address: {address}\nSalt: {salt} ({})", U256::from_be_bytes(salt.0));
        }

        let (salt, address) = results.into_iter().next().unwrap();
        Ok(Create2Output { address, salt })
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
    use std::str::FromStr;

    const DEPLOYER: &str = "0x4e59b44847b379578588920ca78fbf26c0b4956c";

    #[test]
    fn basic_create2() {
        let mk_args = |args: &[&str]| {
            Create2Args::parse_from(["foundry-cli", "--init-code-hash=0x0000000000000000000000000000000000000000000000000000000000000000"].iter().chain(args))
        };

        // even hex chars
        let args = mk_args(&["--starts-with", "aa"]);
        let create2_out = args.run().unwrap();
        assert!(format!("{:x}", create2_out.address).starts_with("aa"));

        let args = mk_args(&["--ends-with", "bb"]);
        let create2_out = args.run().unwrap();
        assert!(format!("{:x}", create2_out.address).ends_with("bb"));

        // odd hex chars
        let args = mk_args(&["--starts-with", "aaa"]);
        let create2_out = args.run().unwrap();
        assert!(format!("{:x}", create2_out.address).starts_with("aaa"));

        let args = mk_args(&["--ends-with", "bbb"]);
        let create2_out = args.run().unwrap();
        assert!(format!("{:x}", create2_out.address).ends_with("bbb"));

        // even hex chars with 0x prefix
        let args = mk_args(&["--starts-with", "0xaa"]);
        let create2_out = args.run().unwrap();
        assert!(format!("{:x}", create2_out.address).starts_with("aa"));

        // odd hex chars with 0x prefix
        let args = mk_args(&["--starts-with", "0xaaa"]);
        let create2_out = args.run().unwrap();
        assert!(format!("{:x}", create2_out.address).starts_with("aaa"));

        // check fails on wrong chars
        let args = mk_args(&["--starts-with", "0xerr"]);
        let create2_out = args.run();
        assert!(create2_out.is_err());

        // check fails on wrong x prefixed string provided
        let args = mk_args(&["--starts-with", "x00"]);
        let create2_out = args.run();
        assert!(create2_out.is_err());
    }

    #[test]
    fn matches_pattern() {
        let args = Create2Args::parse_from([
            "foundry-cli",
            "--init-code-hash=0x0000000000000000000000000000000000000000000000000000000000000000",
            "--matching=0xbbXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
        ]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        assert!(format!("{address:x}").starts_with("bb"));
    }

    #[test]
    fn create2_init_code() {
        let init_code = "00";
        let args =
            Create2Args::parse_from(["foundry-cli", "--starts-with=cc", "--init-code", init_code]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        assert!(format!("{address:x}").starts_with("cc"));
        let salt = create2_out.salt;
        let deployer = Address::from_str(DEPLOYER).unwrap();
        assert_eq!(address, deployer.create2_from_code(salt, hex::decode(init_code).unwrap()));
    }

    #[test]
    fn create2_init_code_hash() {
        let init_code_hash = "bc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a";
        let args = Create2Args::parse_from([
            "foundry-cli",
            "--starts-with=dd",
            "--init-code-hash",
            init_code_hash,
        ]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        assert!(format!("{address:x}").starts_with("dd"));

        let salt = create2_out.salt;
        let deployer = Address::from_str(DEPLOYER).unwrap();

        assert_eq!(
            address,
            deployer
                .create2(salt, B256::from_slice(hex::decode(init_code_hash).unwrap().as_slice()))
        );
    }

    #[test]
    fn create2_caller() {
        let init_code_hash = "bc36789e7a1e281436464229828f817d6612f7b477d66591ff96a9e064bcc98a";
        let args = Create2Args::parse_from([
            "foundry-cli",
            "--starts-with=dd",
            "--init-code-hash",
            init_code_hash,
            "--caller=0x66f9664f97F2b50F62D13eA064982f936dE76657",
        ]);
        let create2_out = args.run().unwrap();
        let address = create2_out.address;
        let salt = create2_out.salt;
        assert!(format!("{address:x}").starts_with("dd"));
        assert!(format!("{salt:x}").starts_with("66f9664f97f2b50f62d13ea064982f936de76657"));
    }
}
