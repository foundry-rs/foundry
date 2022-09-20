//! vanity subcommand
use crate::cmd::Cmd;
use cast::SimpleCast;
use clap::Parser;
use ethers::{
    core::{k256::ecdsa::SigningKey, rand::thread_rng},
    prelude::{LocalWallet, Signer},
    types::{H160, U256},
    utils::{get_contract_address, secret_key_to_address},
};
use rayon::prelude::*;
use regex::Regex;
use std::time::Instant;

#[derive(Debug, Clone, Parser)]
pub struct VanityArgs {
    #[clap(
        long,
        help = "Prefix for the vanity address.",
        required_unless_present = "ends-with",
        validator = hex_address_validator(),
        value_name = "HEX"
    )]
    pub starts_with: Option<String>,
    #[clap(long, help = "Suffix for the vanity address.", validator = hex_address_validator(), value_name = "HEX")]
    pub ends_with: Option<String>,
    #[clap(
        long,
        help = "Generate a vanity contract address created by the generated keypair with the specified nonce.",
        value_name = "NONCE"
    )]
    pub nonce: Option<u64>, /* 2^64-1 is max possible nonce per https://eips.ethereum.org/EIPS/eip-2681 */
}

impl Cmd for VanityArgs {
    type Output = LocalWallet;

    fn run(self) -> eyre::Result<Self::Output> {
        let Self { starts_with, ends_with, nonce } = self;
        let mut left_exact_hex = None;
        let mut left_regex = None;
        let mut right_exact_hex = None;
        let mut right_regex = None;

        if let Some(prefix) = starts_with {
            if let Ok(decoded) = hex::decode(prefix.as_bytes()) {
                left_exact_hex = Some(decoded)
            } else {
                left_regex = Some(Regex::new(&format!(r"^{}", prefix))?);
            }
        }

        if let Some(suffix) = ends_with {
            if let Ok(decoded) = hex::decode(suffix.as_bytes()) {
                right_exact_hex = Some(decoded)
            } else {
                right_regex = Some(Regex::new(&format!(r"{}$", suffix))?);
            }
        }

        macro_rules! find_vanity {
            ($m:ident, $nonce: ident) => {
                if let Some(nonce) = $nonce {
                    find_vanity_address_with_nonce($m, nonce)
                } else {
                    find_vanity_address($m)
                }
            };
        }

        println!("Starting to generate vanity address...");
        let timer = Instant::now();

        let wallet = match (left_exact_hex, left_regex, right_exact_hex, right_regex) {
            (Some(left), _, Some(right), _) => {
                let matcher = HexMatcher { left, right };
                find_vanity!(matcher, nonce)
            }
            (Some(left), _, _, Some(right)) => {
                let matcher = LeftExactRightRegexMatcher { left, right };
                find_vanity!(matcher, nonce)
            }
            (_, Some(left), _, Some(right)) => {
                let matcher = RegexMatcher { left, right };
                find_vanity!(matcher, nonce)
            }
            (_, Some(left), Some(right), _) => {
                let matcher = LeftRegexRightExactMatcher { left, right };
                find_vanity!(matcher, nonce)
            }
            (Some(left), None, None, None) => {
                let matcher = LeftHexMatcher { left };
                find_vanity!(matcher, nonce)
            }
            (None, None, Some(right), None) => {
                let matcher = RightHexMatcher { right };
                find_vanity!(matcher, nonce)
            }
            (None, Some(re), None, None) => {
                let matcher = SingleRegexMatcher { re };
                find_vanity!(matcher, nonce)
            }
            (None, None, None, Some(re)) => {
                let matcher = SingleRegexMatcher { re };
                find_vanity!(matcher, nonce)
            }
            _ => unreachable!(),
        }
        .expect("failed to generate vanity wallet");

        println!(
            "Successfully found vanity address in {} seconds.{}{}\nAddress: {}\nPrivate Key: 0x{}",
            timer.elapsed().as_secs(),
            if nonce.is_some() { "\nContract address: " } else { "" },
            if nonce.is_some() {
                SimpleCast::to_checksum_address(&get_contract_address(
                    wallet.address(),
                    nonce.unwrap(),
                ))
            } else {
                "".to_string()
            },
            SimpleCast::to_checksum_address(&wallet.address()),
            hex::encode(wallet.signer().to_bytes()),
        );

        Ok(wallet)
    }
}

type _Wallet = (SigningKey, H160);

pub fn find_vanity_address<T: VanityMatcher>(m: T) -> Option<LocalWallet> {
    wallet_generator().find_any(matcher(m)).map(|(key, _)| key.into())
}

pub fn find_vanity_address_with_nonce<T: VanityMatcher>(m: T, nonce: u64) -> Option<LocalWallet> {
    let nonce: U256 = nonce.into();
    wallet_generator().find_any(nonce_matcher(m, nonce)).map(|(key, _)| key.into())
}

/// Creates a matcher function.
#[inline]
pub fn matcher<T: VanityMatcher>(m: T) -> impl Fn(&_Wallet) -> bool {
    move |(_, addr)| m.is_match(addr)
}

/// Creates a nonce matcher function.
#[inline]
pub fn nonce_matcher<T: VanityMatcher>(m: T, nonce: U256) -> impl Fn(&_Wallet) -> bool {
    move |(_, addr)| {
        let contract_addr = get_contract_address(*addr, nonce);
        m.is_match(&contract_addr)
    }
}

/// Returns an infinite iterator that calls [generate_wallet].
#[inline]
pub fn wallet_generator() -> impl ParallelIterator<Item = _Wallet> {
    std::iter::repeat(0u8).par_bridge().map(|_| generate_wallet())
}

/// Creates a Wallet and derives its Ethereum address using [thread_rng].
pub fn generate_wallet() -> _Wallet {
    let key = SigningKey::random(&mut thread_rng());
    let address = secret_key_to_address(&key);
    (key, address)
}

/// A trait to match vanity addresses
pub trait VanityMatcher: Send + Sync {
    fn is_match(&self, addr: &H160) -> bool;
}

/// A matcher that checks for if an address starts or ends with certain hex
pub struct HexMatcher {
    pub left: Vec<u8>,
    pub right: Vec<u8>,
}

impl VanityMatcher for HexMatcher {
    #[inline]
    fn is_match(&self, addr: &H160) -> bool {
        let bytes = addr.as_bytes();
        bytes.starts_with(&self.left) && bytes.ends_with(&self.right)
    }
}

pub struct LeftHexMatcher {
    pub left: Vec<u8>,
}

impl VanityMatcher for LeftHexMatcher {
    #[inline]
    fn is_match(&self, addr: &H160) -> bool {
        let bytes = addr.as_bytes();
        bytes.starts_with(&self.left)
    }
}

pub struct RightHexMatcher {
    pub right: Vec<u8>,
}

impl VanityMatcher for RightHexMatcher {
    #[inline]
    fn is_match(&self, addr: &H160) -> bool {
        let bytes = addr.as_bytes();
        bytes.ends_with(&self.right)
    }
}

pub struct LeftExactRightRegexMatcher {
    pub left: Vec<u8>,
    pub right: Regex,
}

impl VanityMatcher for LeftExactRightRegexMatcher {
    #[inline]
    fn is_match(&self, addr: &H160) -> bool {
        let bytes = addr.as_bytes();
        bytes.starts_with(&self.left) && self.right.is_match(&hex::encode(bytes))
    }
}

pub struct LeftRegexRightExactMatcher {
    pub left: Regex,
    pub right: Vec<u8>,
}

impl VanityMatcher for LeftRegexRightExactMatcher {
    #[inline]
    fn is_match(&self, addr: &H160) -> bool {
        let bytes = addr.as_bytes();
        bytes.ends_with(&self.right) && self.left.is_match(&hex::encode(bytes))
    }
}

/// Matches a single regex
pub struct SingleRegexMatcher {
    pub re: Regex,
}

impl VanityMatcher for SingleRegexMatcher {
    #[inline]
    fn is_match(&self, addr: &H160) -> bool {
        let addr = hex::encode(addr.as_ref());
        self.re.is_match(&addr)
    }
}

/// Matches left and right regex
pub struct RegexMatcher {
    pub left: Regex,
    pub right: Regex,
}

impl VanityMatcher for RegexMatcher {
    #[inline]
    fn is_match(&self, addr: &H160) -> bool {
        let addr = hex::encode(addr.as_ref());
        self.left.is_match(&addr) && self.right.is_match(&addr)
    }
}

pub fn hex_address_validator() -> impl FnMut(&str) -> eyre::Result<()> {
    move |v: &str| -> eyre::Result<()> {
        if v.len() > 40 {
            eyre::bail!("vanity patterns length exceeded. cannot be more than 40 characters")
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_simple_vanity_start() {
        let args: VanityArgs = VanityArgs::parse_from(["foundry-cli", "--starts-with", "00"]);
        let wallet = args.run().unwrap();
        let addr = wallet.address();
        let addr = format!("{addr:x}");
        assert!(addr.starts_with("00"));
    }

    #[test]
    fn find_simple_vanity_start2() {
        let args: VanityArgs = VanityArgs::parse_from(["foundry-cli", "--starts-with", "9"]);
        let wallet = args.run().unwrap();
        let addr = wallet.address();
        let addr = format!("{addr:x}");
        assert!(addr.starts_with('9'));
    }

    #[test]
    fn find_simple_vanity_end() {
        let args: VanityArgs = VanityArgs::parse_from(["foundry-cli", "--ends-with", "00"]);
        let wallet = args.run().unwrap();
        let addr = wallet.address();
        let addr = format!("{addr:x}");
        assert!(addr.ends_with("00"));
    }
}
