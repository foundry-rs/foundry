use cast::SimpleCast;
use clap::{builder::TypedValueParser, Parser};
use ethers::{
    core::{k256::ecdsa::SigningKey, rand::thread_rng},
    prelude::{LocalWallet, Signer},
    utils::secret_key_to_address,
};
use alloy_primitives::{Address, U256};
use eyre::Result;

use foundry_utils::types::ToAlloy;
use rayon::iter::{self, ParallelIterator};
use regex::Regex;
use std::time::Instant;

/// Type alias for the result of [generate_wallet].
pub type GeneratedWallet = (SigningKey, Address);

/// CLI arguments for `cast wallet vanity`.
#[derive(Debug, Clone, Parser)]
pub struct VanityArgs {
    /// Prefix for the vanity address.
    #[clap(
        long,
        required_unless_present = "ends_with",
        value_parser = HexAddressValidator,
        value_name = "HEX"
    )]
    pub starts_with: Option<String>,

    /// Suffix for the vanity address.
    #[clap(long, value_parser = HexAddressValidator, value_name = "HEX")]
    pub ends_with: Option<String>,

    // 2^64-1 is max possible nonce per [eip-2681](https://eips.ethereum.org/EIPS/eip-2681).
    /// Generate a vanity contract address created by the generated keypair with the specified
    /// nonce.
    #[clap(long)]
    pub nonce: Option<u64>,
}

impl VanityArgs {
    pub fn run(self) -> Result<LocalWallet> {
        let Self { starts_with, ends_with, nonce } = self;
        let mut left_exact_hex = None;
        let mut left_regex = None;
        let mut right_exact_hex = None;
        let mut right_regex = None;

        if let Some(prefix) = starts_with {
            if let Ok(decoded) = hex::decode(prefix.as_bytes()) {
                left_exact_hex = Some(decoded)
            } else {
                left_regex = Some(Regex::new(&format!(r"^{prefix}"))?);
            }
        }

        if let Some(suffix) = ends_with {
            if let Ok(decoded) = hex::decode(suffix.as_bytes()) {
                right_exact_hex = Some(decoded)
            } else {
                right_regex = Some(Regex::new(&format!(r"{suffix}$"))?);
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
                wallet.address().to_alloy().create(nonce.unwrap()).to_checksum(None)
            } else {
                "".to_string()
            },
            SimpleCast::to_checksum_address(&wallet.address()),
            hex::encode(wallet.signer().to_bytes()),
        );

        Ok(wallet)
    }
}

/// Generates random wallets until `matcher` matches the wallet address, returning the wallet.
pub fn find_vanity_address<T: VanityMatcher>(matcher: T) -> Option<LocalWallet> {
    wallet_generator().find_any(create_matcher(matcher)).map(|(key, _)| key.into())
}

/// Generates random wallets until `matcher` matches the contract address created at `nonce`,
/// returning the wallet.
pub fn find_vanity_address_with_nonce<T: VanityMatcher>(
    matcher: T,
    nonce: u64,
) -> Option<LocalWallet> {
    let nonce: U256 = U256::from(nonce);
    wallet_generator().find_any(create_nonce_matcher(matcher, nonce)).map(|(key, _)| key.into())
}

/// Creates a nonce matcher function, which takes a reference to a [GeneratedWallet] and returns
/// whether it found a match or not by using `matcher`.
#[inline]
pub fn create_matcher<T: VanityMatcher>(matcher: T) -> impl Fn(&GeneratedWallet) -> bool {
    move |(_, addr)| matcher.is_match(addr)
}

/// Creates a nonce matcher function, which takes a reference to a [GeneratedWallet] and a nonce and
/// returns whether it found a match or not by using `matcher`.
#[inline]
pub fn create_nonce_matcher<T: VanityMatcher>(
    matcher: T,
    nonce: U256,
) -> impl Fn(&GeneratedWallet) -> bool {
    move |(_, addr)| {
        let contract_addr = (*addr).create(nonce.to());
        matcher.is_match(&contract_addr)
    }
}

/// Returns an infinite parallel iterator which yields a [GeneratedWallet].
#[inline]
pub fn wallet_generator() -> iter::Map<iter::Repeat<()>, fn(()) -> GeneratedWallet> {
    iter::repeat(()).map(|_| generate_wallet())
}

/// Generates a random K-256 signing key and derives its Ethereum address.
pub fn generate_wallet() -> GeneratedWallet {
    let key = SigningKey::random(&mut thread_rng());
    let address = secret_key_to_address(&key).to_alloy();
    (key, address)
}

/// A trait to match vanity addresses.
pub trait VanityMatcher: Send + Sync {
    fn is_match(&self, addr: &Address) -> bool;
}

/// Matches start and end hex.
pub struct HexMatcher {
    pub left: Vec<u8>,
    pub right: Vec<u8>,
}

impl VanityMatcher for HexMatcher {
    #[inline]
    fn is_match(&self, addr: &Address) -> bool {
        let bytes = addr.as_slice();
        bytes.starts_with(&self.left) && bytes.ends_with(&self.right)
    }
}

/// Matches only start hex.
pub struct LeftHexMatcher {
    pub left: Vec<u8>,
}

impl VanityMatcher for LeftHexMatcher {
    #[inline]
    fn is_match(&self, addr: &Address) -> bool {
        let bytes = addr.as_slice();
        bytes.starts_with(&self.left)
    }
}

/// Matches only end hex.
pub struct RightHexMatcher {
    pub right: Vec<u8>,
}

impl VanityMatcher for RightHexMatcher {
    #[inline]
    fn is_match(&self, addr: &Address) -> bool {
        let bytes = addr.as_slice();
        bytes.ends_with(&self.right)
    }
}

/// Matches start hex and end regex.
pub struct LeftExactRightRegexMatcher {
    pub left: Vec<u8>,
    pub right: Regex,
}

impl VanityMatcher for LeftExactRightRegexMatcher {
    #[inline]
    fn is_match(&self, addr: &Address) -> bool {
        let bytes = addr.as_slice();
        bytes.starts_with(&self.left) && self.right.is_match(&hex::encode(bytes))
    }
}

/// Matches start regex and end hex.
pub struct LeftRegexRightExactMatcher {
    pub left: Regex,
    pub right: Vec<u8>,
}

impl VanityMatcher for LeftRegexRightExactMatcher {
    #[inline]
    fn is_match(&self, addr: &Address) -> bool {
        let bytes = addr.as_slice();
        bytes.ends_with(&self.right) && self.left.is_match(&hex::encode(bytes))
    }
}

/// Matches a single regex.
pub struct SingleRegexMatcher {
    pub re: Regex,
}

impl VanityMatcher for SingleRegexMatcher {
    #[inline]
    fn is_match(&self, addr: &Address) -> bool {
        let addr = hex::encode(addr);
        self.re.is_match(&addr)
    }
}

/// Matches start and end regex.
pub struct RegexMatcher {
    pub left: Regex,
    pub right: Regex,
}

impl VanityMatcher for RegexMatcher {
    #[inline]
    fn is_match(&self, addr: &Address) -> bool {
        let addr = hex::encode(addr);
        self.left.is_match(&addr) && self.right.is_match(&addr)
    }
}

/// Parse 40 byte addresses
#[derive(Copy, Clone, Debug, Default)]
pub struct HexAddressValidator;

impl TypedValueParser for HexAddressValidator {
    type Value = String;

    fn parse_ref(
        &self,
        _cmd: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        if value.len() > 40 {
            return Err(clap::Error::raw(
                clap::error::ErrorKind::InvalidValue,
                "vanity patterns length exceeded. cannot be more than 40 characters",
            ))
        }
        let value = value.to_str().ok_or_else(|| {
            clap::Error::raw(clap::error::ErrorKind::InvalidUtf8, "address must be valid utf8")
        })?;
        Ok(value.to_string())
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
