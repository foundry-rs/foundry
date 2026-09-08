use alloy_primitives::{Address, hex};
use alloy_signer_local::PrivateKeySigner;
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::json::print_json_success;
use foundry_common::{sh_println, shell};
use rayon::iter::{self, ParallelIterator};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

/// CLI arguments for `cast wallet vanity`.
#[derive(Clone, Debug, Parser)]
pub struct VanityArgs {
    /// Prefix regex pattern or hex string.
    #[arg(long, value_name = "PATTERN", required_unless_present = "ends_with")]
    pub starts_with: Option<String>,

    /// Suffix regex pattern or hex string.
    #[arg(long, value_name = "PATTERN")]
    pub ends_with: Option<String>,

    // 2^64-1 is max possible nonce per [eip-2681](https://eips.ethereum.org/EIPS/eip-2681).
    /// Generate a vanity contract address created by the generated keypair with the specified
    /// nonce.
    #[arg(long)]
    pub nonce: Option<u64>,

    /// Path to save the generated vanity contract address to.
    ///
    /// If provided, the generated vanity addresses will appended to a JSON array in the specified
    /// file.
    #[arg(
        long,
        value_hint = clap::ValueHint::FilePath,
        value_name = "PATH",
    )]
    pub save_path: Option<PathBuf>,
}

/// WalletData contains address and private_key information for a wallet.
#[derive(Serialize, Deserialize)]
struct WalletData {
    address: String,
    private_key: String,
}

/// Wallets is a collection of WalletData.
#[derive(Default, Serialize, Deserialize)]
struct Wallets {
    wallets: Vec<WalletData>,
}

impl WalletData {
    fn new(wallet: &PrivateKeySigner) -> Self {
        Self {
            address: wallet.address().to_checksum(None),
            private_key: format!("0x{}", hex::encode(wallet.credential().to_bytes())),
        }
    }
}

impl VanityArgs {
    pub fn run(self) -> Result<PrivateKeySigner> {
        let Self { starts_with, ends_with, nonce, save_path } = self;

        let matcher = Matcher {
            left: starts_with.as_deref().map(|p| parse_pattern(p, true)).transpose()?,
            right: ends_with.as_deref().map(|p| parse_pattern(p, false)).transpose()?,
        };

        sh_status!("Starting to generate vanity address...")?;
        let timer = Instant::now();
        let wallet = find_vanity(&matcher, nonce);

        if let Some(save_path) = save_path {
            save_wallet_to_file(&wallet, &save_path)?;
        }

        let contract_address = nonce.map(|nonce| wallet.address().create(nonce).to_checksum(None));
        let WalletData { address, private_key } = WalletData::new(&wallet);

        if shell::is_json() {
            print_json_success(json!({
                "address": address,
                "private_key": private_key,
                "contract_address": contract_address,
            }))?;
        } else {
            sh_println!(
                "Successfully found vanity address in {:.3} seconds.{}{}\nAddress: {}\nPrivate Key: {}",
                timer.elapsed().as_secs_f64(),
                if contract_address.is_some() { "\nContract address: " } else { "" },
                contract_address.unwrap_or_default(),
                address,
                private_key,
            )?;
        }

        Ok(wallet)
    }
}

/// Saves the specified `wallet` to a 'vanity_addresses.json' file at the given `save_path`.
/// If the file exists, the wallet data is appended to the existing content;
/// otherwise, a new file is created.
fn save_wallet_to_file(wallet: &PrivateKeySigner, path: &Path) -> Result<()> {
    let mut wallets = if path.exists() {
        let data = fs::read_to_string(path)?;
        if data.trim().is_empty() {
            Wallets::default()
        } else {
            serde_json::from_str::<Wallets>(&data)
                .wrap_err_with(|| format!("failed to parse wallet file {}", path.display()))?
        }
    } else {
        Wallets::default()
    };

    wallets.wallets.push(WalletData::new(wallet));

    let contents = serde_json::to_string_pretty(&wallets)?;
    let mut options = fs::File::options();
    options.write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options.open(path)?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.set_len(0)?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

/// Generates random wallets in parallel until `matcher` matches the wallet address, or the
/// contract address it creates at `nonce` when one is given.
fn find_vanity(matcher: &Matcher, nonce: Option<u64>) -> PrivateKeySigner {
    iter::repeat(())
        .map(|()| PrivateKeySigner::random())
        .find_any(|wallet| {
            let address = wallet.address();
            matcher.is_match(&nonce.map_or(address, |nonce| address.create(nonce)))
        })
        .expect("infinite iterator")
}

/// A vanity pattern: an exact byte prefix/suffix, or a regex over the lowercase hex address.
#[derive(Debug)]
enum Pattern {
    Exact(Vec<u8>),
    Re(Regex),
}

/// Optional start and end patterns an address must satisfy.
struct Matcher {
    left: Option<Pattern>,
    right: Option<Pattern>,
}

impl Matcher {
    fn is_match(&self, addr: &Address) -> bool {
        let bytes = addr.as_slice();
        let mut encoded = None;
        let mut matches = |pattern: &Pattern, exact: fn(&[u8], &[u8]) -> bool| match pattern {
            Pattern::Exact(hex) => exact(bytes, hex),
            Pattern::Re(re) => re.is_match(encoded.get_or_insert_with(|| hex::encode(bytes))),
        };
        self.left.as_ref().is_none_or(|p| matches(p, <[u8]>::starts_with))
            && self.right.as_ref().is_none_or(|p| matches(p, <[u8]>::ends_with))
    }
}

fn parse_pattern(pattern: &str, is_start: bool) -> Result<Pattern> {
    let pattern =
        pattern.strip_prefix("0x").or_else(|| pattern.strip_prefix("0X")).unwrap_or(pattern);
    if pattern.is_empty() {
        return Err(eyre::eyre!("Vanity pattern cannot be empty"));
    }

    let is_hex = pattern.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_hex && pattern.len() > 40 {
        return Err(eyre::eyre!("Hex pattern must be less than 20 bytes"));
    }

    if let Ok(decoded) = hex::decode(pattern) {
        return Ok(Pattern::Exact(decoded));
    }
    // a non regex literal containing non-hex characters can never match
    if !is_hex && pattern.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(eyre::eyre!("Pattern contains non-hex characters and can never match"));
    }
    let (prefix, suffix) = if is_start { ("^", "") } else { ("", "$") };
    let pattern = if is_hex { pattern.to_ascii_lowercase() } else { pattern.to_string() };
    Ok(Pattern::Re(Regex::new(&format!("{prefix}{pattern}{suffix}"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(pattern: &str, is_start: bool) -> Matcher {
        let pattern = parse_pattern(pattern, is_start).unwrap();
        if is_start {
            Matcher { left: Some(pattern), right: None }
        } else {
            Matcher { left: None, right: Some(pattern) }
        }
    }

    fn address(index: usize, byte: u8) -> Address {
        let mut bytes = [0; 20];
        bytes[index] = byte;
        Address::from(bytes)
    }

    #[test]
    fn save_path() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let args: VanityArgs = VanityArgs::parse_from([
            "foundry-cli",
            "--starts-with",
            "00",
            "--save-path",
            tmp.path().to_str().unwrap(),
        ]);
        let wallet = args.run().unwrap();
        assert!(wallet.address().as_slice().starts_with(&[0]));
        let s = fs::read_to_string(tmp.path()).unwrap();
        let wallets: Wallets = serde_json::from_str(&s).unwrap();
        assert!(!wallets.wallets.is_empty());
    }

    #[test]
    fn malformed_wallet_file_is_not_overwritten() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let original = "{\"wallets\":[";
        fs::write(tmp.path(), original).unwrap();

        let err = save_wallet_to_file(&PrivateKeySigner::random(), tmp.path()).unwrap_err();

        assert!(err.to_string().contains("failed to parse wallet file"));
        assert_eq!(fs::read_to_string(tmp.path()).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn wallet_file_is_owner_only() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("wallets.json");
        save_wallet_to_file(&PrivateKeySigner::random(), &path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        save_wallet_to_file(&PrivateKeySigner::random(), &path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn parse_patterns() {
        // odd-length hex is matched case-insensitively as a regex
        assert!(matches!(parse_pattern("A", true).unwrap(), Pattern::Re(_)));
        assert!(single("A", true).is_match(&address(0, 0xa0)));
        assert!(single("A", false).is_match(&address(19, 0x0a)));
        assert!(single("0x9", true).is_match(&address(0, 0x90)));

        // 0x/0X prefixes are stripped from exact hex patterns
        for prefixed in ["0xdead", "0Xdead"] {
            let Pattern::Exact(bytes) = parse_pattern(prefixed, true).unwrap() else {
                panic!("expected an exact hex pattern");
            };
            assert_eq!(bytes, hex::decode("dead").unwrap());
        }
        let mut matching = [0; 20];
        matching[..2].copy_from_slice(&[0xde, 0xad]);
        assert!(single("0xdead", true).is_match(&Address::from(matching)));

        // regex patterns stay supported
        assert!(parse_pattern("a.c", true).is_ok());

        // exact suffixes match the end of the address only
        assert!(matches!(parse_pattern("00", false).unwrap(), Pattern::Exact(_)));
        assert!(single("00", false).is_match(&address(0, 0xff)));
        assert!(!single("00", false).is_match(&address(19, 0x01)));
        assert!(!single("dead", true).is_match(&address(19, 0xde)));
    }

    #[test]
    fn reject_invalid_patterns() {
        for (pattern, err) in [
            (&*"1".repeat(41), "Hex pattern must be less than 20 bytes"),
            ("0x", "Vanity pattern cannot be empty"),
            // non-hex chars can never appear in a hex address
            ("zzz", "Pattern contains non-hex characters and can never match"),
            ("foobar", "Pattern contains non-hex characters and can never match"),
        ] {
            assert_eq!(parse_pattern(pattern, true).unwrap_err().to_string(), err);
        }
    }
}
