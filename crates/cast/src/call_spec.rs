//! Call specification parsing for batch transactions.
//!
//! Parses call specs in the format: `to[:<value>][:<sig>[:<args>]]` or `to[:<value>][:<0xrawdata>]`
//!
//! Examples:
//! - `0x123` - Just an address (empty call)
//! - `0x123:0.1ether` - ETH transfer
//! - `0x123::transfer(address,uint256):0x789,1000` - Contract call with signature
//! - `0x123::0xabcdef` - Contract call with raw calldata

use alloy_network::Network;
use alloy_primitives::{Address, Bytes, U256, hex};
use alloy_provider::Provider;
use eyre::{Result, WrapErr, eyre};
use foundry_cli::utils::parse_function_args;
use foundry_config::Chain;
use std::str::FromStr;
use tempo_primitives::transaction::Call;

/// A parsed call specification for batch transactions.
#[derive(Debug, Clone)]
pub struct CallSpec {
    /// Target address (required)
    pub to: Address,
    /// ETH value to send (optional, defaults to 0)
    pub value: U256,
    /// Function signature, e.g., "transfer(address,uint256)" (optional)
    pub sig: Option<String>,
    /// Function arguments (optional)
    pub args: Vec<String>,
    /// Raw calldata if provided instead of sig+args (optional)
    pub data: Option<Bytes>,
}

impl CallSpec {
    /// Parse a call spec string.
    ///
    /// Format: `to[:<value>][:<sig>[:<args>]]` or `to[:<value>][:<0xrawdata>]`
    ///
    /// The delimiter is `:` but we need to be careful about:
    /// - Colons in function signatures (none expected)
    /// - Colons in hex addresses (none expected)
    /// - We use double-colon `::` to separate value from sig/data when value is empty
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(eyre!("Empty call specification"));
        }

        // Split by `:` but handle `::` for empty value
        let parts: Vec<&str> = s.split(':').collect();

        if parts.is_empty() {
            return Err(eyre!("Invalid call specification: {}", s));
        }

        // First part is always the address
        let to = Address::from_str(parts[0])
            .map_err(|e| eyre!("Invalid address '{}': {}", parts[0], e))?;

        let mut value = U256::ZERO;
        let mut sig = None;
        let mut args = Vec::new();
        let mut data = None;

        // Parse remaining parts
        // Pattern: to:value:sig:args or to::sig:args (empty value) or to:value:0xdata
        let mut idx = 1;

        // Check for value (non-empty, not starting with 0x unless it's a number)
        if idx < parts.len() {
            let part = parts[idx];
            if !part.is_empty() && !part.starts_with("0x") && !part.contains('(') {
                // This looks like a value
                value = parse_ether_or_wei(part)?;
                idx += 1;
            } else if part.is_empty() {
                // Empty value (::), skip
                idx += 1;
            }
        }

        // Check for sig/data
        if idx < parts.len() {
            let part = parts[idx];
            if part.starts_with("0x") {
                // Raw calldata
                data = Some(Bytes::from(
                    hex::decode(part).map_err(|e| eyre!("Invalid hex data '{}': {}", part, e))?,
                ));
            } else if !part.is_empty() {
                // Function signature
                sig = Some(part.to_string());
                idx += 1;

                // Collect remaining parts as args (comma-separated in the last part)
                if idx < parts.len() {
                    let args_str = parts[idx..].join(":");
                    args = args_str.split(',').map(|s| s.trim().to_string()).collect();
                }
            }
        }

        Ok(Self { to, value, sig, args, data })
    }

    /// Resolves this spec into a [`Call`], encoding function arguments if needed.
    /// `i` is the 0-based index of this call; displayed as `i + 1` in error messages.
    pub async fn resolve<N: Network, P: Provider<N>>(
        &self,
        i: usize,
        chain: Chain,
        provider: &P,
        etherscan_api_key: Option<&str>,
    ) -> Result<Call> {
        let input = if let Some(data) = &self.data {
            data.clone()
        } else if let Some(sig) = &self.sig {
            let (encoded, _) = parse_function_args(
                sig,
                self.args.clone(),
                Some(self.to),
                chain,
                provider,
                etherscan_api_key,
            )
            .await
            .map_err(|e| eyre!("Failed to encode call {}: {e}", i + 1))?;
            Bytes::from(encoded)
        } else {
            Bytes::new()
        };
        Ok(Call { to: self.to.into(), value: self.value, input })
    }
}

impl FromStr for CallSpec {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// Parse a value string that can be in ether notation (e.g., "0.1ether") or raw wei.
fn parse_ether_or_wei(s: &str) -> Result<U256> {
    // Use alloy's DynSolType coercion which handles "1ether", "1gwei", "1000" etc.
    if s.starts_with("0x") || s.starts_with("0X") {
        U256::from_str(s).map_err(|e| eyre!("Invalid hex value '{}': {}", s, e))
    } else {
        alloy_dyn_abi::DynSolType::coerce_str(&alloy_dyn_abi::DynSolType::Uint(256), s)
            .wrap_err_with(|| format!("Invalid value '{s}'"))?
            .as_uint()
            .map(|(v, _)| v)
            .ok_or_else(|| eyre!("Could not parse value '{}'", s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_address_only() {
        let spec = CallSpec::parse("0x1234567890123456789012345678901234567890").unwrap();
        assert_eq!(
            spec.to,
            "0x1234567890123456789012345678901234567890".parse::<Address>().unwrap()
        );
        assert_eq!(spec.value, U256::ZERO);
        assert!(spec.sig.is_none());
        assert!(spec.args.is_empty());
        assert!(spec.data.is_none());
    }

    #[test]
    fn test_parse_with_value() {
        let spec = CallSpec::parse("0x1234567890123456789012345678901234567890:1ether").unwrap();
        assert_eq!(spec.value, parse_ether_or_wei("1ether").unwrap());
        assert!(spec.sig.is_none());
    }

    #[test]
    fn test_parse_hex_value() {
        assert_eq!(parse_ether_or_wei("0x10").unwrap(), U256::from(16));
        assert_eq!(parse_ether_or_wei("0X10").unwrap(), U256::from(16));
    }

    #[test]
    fn test_parse_with_sig() {
        let spec = CallSpec::parse(
            "0x1234567890123456789012345678901234567890::transfer(address,uint256):0xabc,1000",
        )
        .unwrap();
        assert_eq!(spec.value, U256::ZERO);
        assert_eq!(spec.sig, Some("transfer(address,uint256)".to_string()));
        assert_eq!(spec.args, vec!["0xabc", "1000"]);
    }

    #[test]
    fn test_parse_with_value_and_sig() {
        let spec = CallSpec::parse(
            "0x1234567890123456789012345678901234567890:0.5ether:transfer(address,uint256):0xabc,1000",
        )
        .unwrap();
        assert_eq!(spec.value, parse_ether_or_wei("0.5ether").unwrap());
        assert_eq!(spec.sig, Some("transfer(address,uint256)".to_string()));
    }

    #[test]
    fn test_parse_with_raw_data() {
        let spec = CallSpec::parse("0x1234567890123456789012345678901234567890::0xabcdef").unwrap();
        assert_eq!(spec.value, U256::ZERO);
        assert!(spec.sig.is_none());
        assert_eq!(spec.data, Some(Bytes::from(hex::decode("abcdef").unwrap())));
    }
}
