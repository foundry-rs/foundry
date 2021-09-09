use ethers::{types::Address, utils};
use eyre::{Context, Result};
use rustc_hex::ToHex;
use std::str::FromStr;

// TODO: SethContract with common contract initializers? Same for SethProviders?

#[derive(Default)]
pub struct Seth {}

impl Seth {
    pub fn new() -> Self {
        Self {}
    }

    /// Converts ASCII text input to hex
    ///
    /// ```
    /// use dapptools::seth::Seth;
    ///
    /// let bin = Seth::from_ascii("yo");
    /// assert_eq!(bin, "0x796f")
    /// ```
    pub fn from_ascii(s: &str) -> String {
        let s: String = s.as_bytes().to_hex();
        format!("0x{}", s)
    }

    /// Converts an Ethereum address to its checksum format
    /// according to [EIP-55](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-55.md)
    ///
    /// ```
    /// use dapptools::seth::Seth;
    ///
    /// # fn main() -> eyre::Result<()> {
    /// let addr = Seth::to_checksum_address("0xb7e390864a90b7b923c9f9310c6f98aafe43f707")?;
    /// assert_eq!(addr, "0xB7e390864a90b7b923C9f9310C6F98aafE43F707");
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_checksum_address(s: &str) -> Result<String> {
        let addr = Address::from_str(s).context("could not parse address for checksum")?;
        Ok(utils::to_checksum(&addr, None))
    }
}
