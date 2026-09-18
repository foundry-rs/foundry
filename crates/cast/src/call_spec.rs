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
use foundry_cli::utils::{parse_ether_value, parse_function_args};
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
    /// Format: `to[:<value>][:<sig>[:<args>]]` or `to[:<value>][:<0xrawdata>]`. A double colon
    /// (`::`) separates the address from the sig/data when the value is omitted.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(eyre!("Empty call specification"));
        }

        let parts: Vec<&str> = s.split(':').collect();
        let to = Address::from_str(parts[0])
            .map_err(|e| eyre!("Invalid address '{}': {}", parts[0], e))?;
        let mut spec = Self { to, value: U256::ZERO, sig: None, args: Vec::new(), data: None };

        // The first field is the value unless it is empty, a signature, or a terminal lowercase
        // hex field, which is raw calldata.
        let mut rest = &parts[1..];
        if let Some((part, tail)) = rest.split_first() {
            if part.is_empty() {
                rest = tail;
            } else if (!part.starts_with("0x") || !tail.is_empty()) && !part.contains('(') {
                spec.value =
                    parse_ether_value(part).wrap_err_with(|| format!("Invalid value '{part}'"))?;
                rest = tail;
            }
        }

        match rest.split_first() {
            Some((part, tail)) if part.starts_with("0x") => {
                let decoded =
                    hex::decode(part).map_err(|e| eyre!("Invalid hex data '{}': {}", part, e))?;
                eyre::ensure!(tail.is_empty(), "Unexpected trailing field(s) after raw calldata");
                spec.data = Some(Bytes::from(decoded));
            }
            Some((part, tail)) if !part.is_empty() => {
                spec.sig = Some(part.to_string());
                if !tail.is_empty() {
                    // Args are comma-separated; rejoin any colons that were split off.
                    let args_str = tail.join(":");
                    spec.args = split_call_args(&args_str);
                }
            }
            _ => {}
        }

        Ok(spec)
    }

    /// Resolves this spec into a [`Call`], encoding function arguments if needed.
    /// `i` is the 0-based index of this call; displayed as `i + 1` in error messages.
    pub async fn resolve<N: Network, P: Provider<N>>(
        &self,
        i: usize,
        chain: Chain,
        provider: &P,
        etherscan_api_key: Option<&str>,
        etherscan_api_url: Option<&str>,
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
                etherscan_api_url,
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

/// Split call arguments on top-level commas, respecting nested parentheses and brackets.
///
/// This ensures that array and tuple arguments containing internal commas are not incorrectly
/// split. For example:
/// - `[1,2]` stays as one argument
/// - `(7,hello),9` splits into `(7,hello)` and `9`
fn split_call_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;

    for (idx, ch) in s.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(s[start..idx].trim().to_string());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    args.push(s[start..].trim().to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_address_and_value() {
        let address = "0x1234567890123456789012345678901234567890";

        let spec = CallSpec::parse(address).unwrap();
        assert_eq!(spec.to, address.parse::<Address>().unwrap());
        assert_eq!(spec.value, U256::ZERO);
        assert!(spec.sig.is_none() && spec.args.is_empty() && spec.data.is_none());

        let spec = CallSpec::parse(&format!("{address}:1ether")).unwrap();
        assert_eq!(spec.value, parse_ether_value("1ether").unwrap());
        assert!(spec.sig.is_none());
    }

    #[test]
    fn test_parse_lowercase_hex_value() {
        let address = "0x1234567890123456789012345678901234567890";

        let spec = CallSpec::parse(&format!("{address}:0x10:deposit()")).unwrap();
        assert_eq!(spec.value, U256::from(16));
        assert_eq!(spec.sig.as_deref(), Some("deposit()"));

        let spec = CallSpec::parse(&format!("{address}:0x10")).unwrap();
        assert_eq!(spec.value, U256::ZERO);
        assert_eq!(spec.data, Some(Bytes::from([0x10])));
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
        assert_eq!(spec.value, parse_ether_value("0.5ether").unwrap());
        assert_eq!(spec.sig, Some("transfer(address,uint256)".to_string()));
    }

    #[test]
    fn test_parse_with_raw_data() {
        let spec = CallSpec::parse("0x1234567890123456789012345678901234567890::0xabcdef").unwrap();
        assert_eq!(spec.value, U256::ZERO);
        assert!(spec.sig.is_none());
        assert_eq!(spec.data, Some(Bytes::from(hex::decode("abcdef").unwrap())));
    }

    #[test]
    fn test_parse_raw_data_rejects_trailing_fields() {
        for spec in [
            "0x1234567890123456789012345678901234567890::0xabcdef:typo",
            "0x1234567890123456789012345678901234567890:1wei:0xabcdef:unexpected",
        ] {
            assert_eq!(
                CallSpec::parse(spec).unwrap_err().to_string(),
                "Unexpected trailing field(s) after raw calldata"
            );
        }
    }

    #[test]
    fn test_parse_array_args() {
        let spec =
            CallSpec::parse("0x1234567890123456789012345678901234567890::foo(uint256[]):[1,2]")
                .unwrap();
        assert_eq!(spec.sig, Some("foo(uint256[])".to_string()));
        assert_eq!(spec.args, vec!["[1,2]"]);

        let spec = CallSpec::parse(
            "0x1234567890123456789012345678901234567890::foo(uint256[][]):[[1,2],[3,4]]",
        )
        .unwrap();
        assert_eq!(spec.sig, Some("foo(uint256[][])".to_string()));
        assert_eq!(spec.args, vec!["[[1,2],[3,4]]"]);
    }

    #[test]
    fn test_parse_tuple_args() {
        let spec = CallSpec::parse(
            "0x1234567890123456789012345678901234567890::foo((uint256,string)):(7,hello)",
        )
        .unwrap();
        assert_eq!(spec.sig, Some("foo((uint256,string))".to_string()));
        assert_eq!(spec.args, vec!["(7,hello)"]);

        let spec = CallSpec::parse(
            "0x1234567890123456789012345678901234567890::foo((uint256,string),uint256):(7,hello),9",
        )
        .unwrap();
        assert_eq!(spec.sig, Some("foo((uint256,string),uint256)".to_string()));
        assert_eq!(spec.args, vec!["(7,hello)", "9"]);
    }

    #[test]
    fn test_parse_nested_structures() {
        let spec = CallSpec::parse(
            "0x1234567890123456789012345678901234567890::foo((uint256[],string)):([(1,2),(3,4)],hello)",
        )
        .unwrap();
        assert_eq!(spec.sig, Some("foo((uint256[],string))".to_string()));
        assert_eq!(spec.args, vec!["([(1,2),(3,4)],hello)"]);
    }

    #[test]
    fn test_split_call_args() {
        assert_eq!(split_call_args("[1,2]"), vec!["[1,2]"]);
        assert_eq!(split_call_args("[[1,2],[3,4]]"), vec!["[[1,2],[3,4]]"]);
        assert_eq!(split_call_args("(7,hello)"), vec!["(7,hello)"]);
        assert_eq!(split_call_args("(7,hello),9"), vec!["(7,hello)", "9"]);
        assert_eq!(split_call_args("1,2,3"), vec!["1", "2", "3"]);
        assert_eq!(split_call_args("(1,2),(3,4)"), vec!["(1,2)", "(3,4)"]);
        assert_eq!(split_call_args("[1,2],[3,4]"), vec!["[1,2]", "[3,4]"]);
    }
}
