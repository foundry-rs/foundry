//! Call specification parsing for batch transactions.
//!
//! Parses call specs in the format: `to[:<value>][:<sig>[:<args>]]` or `to[:<value>][:<0xrawdata>]`
//!
//! Examples:
//! - `0x1234567890123456789012345678901234567890` - Just an address (empty call)
//! - `0x1234567890123456789012345678901234567890:0.1ether` - ETH transfer
//! - `0x1234567890123456789012345678901234567890::transfer(address,uint256):
//!   0x0987654321098765432109876543210987654321,1000` - Contract call with signature
//! - `0x1234567890123456789012345678901234567890::batch(uint256[],(uint256,uint256)): [1,2],(3,4)`
//!   - Array and tuple arguments.
//! - `0x1234567890123456789012345678901234567890::0x123def` - Contract call with raw calldata
//! - `0x1234567890123456789012345678901234567890:1ether:deposit()` - Value + function call

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
    /// Arguments split using the legacy comma-separated grammar, when that differs from `args`.
    legacy_args: Option<Vec<String>>,
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
        let mut spec = Self {
            to,
            value: U256::ZERO,
            sig: None,
            args: Vec::new(),
            legacy_args: None,
            data: None,
        };

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
                    let legacy_args =
                        args_str.split(',').map(|arg| arg.trim().to_string()).collect::<Vec<_>>();
                    if legacy_args != spec.args {
                        spec.legacy_args = Some(legacy_args);
                    }
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
            let mut result = None;
            for args in self.legacy_args.iter().chain(std::iter::once(&self.args)) {
                result = Some(
                    parse_function_args(
                        sig,
                        args.clone(),
                        Some(self.to),
                        chain,
                        provider,
                        etherscan_api_key,
                        etherscan_api_url,
                    )
                    .await,
                );
                if result.as_ref().is_some_and(Result::is_ok) {
                    break;
                }
            }
            let (encoded, _) = result
                .expect("argument candidates are never empty")
                .map_err(|e| eyre!("Failed to encode call {}: {e}", i + 1))?;
            Bytes::from(encoded)
        } else {
            Bytes::new()
        };
        Ok(Call { to: self.to.into(), value: self.value, input })
    }
}

/// Split call arguments on top-level commas, respecting nested parentheses, brackets, and quoted
/// strings.
///
/// This ensures that array and tuple arguments containing internal commas are not incorrectly
/// split. For example:
/// - `[1,2]` stays as one argument
/// - `(7,hello),9` splits into `(7,hello)` and `9`
fn split_call_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut quote = None;
    let mut at_value_start = true;

    for (idx, ch) in s.char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '\'' | '"' if at_value_start => {
                quote = Some(ch);
                at_value_start = false;
            }
            '(' | '[' => {
                depth += 1;
                at_value_start = true;
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                at_value_start = false;
            }
            ',' if depth == 0 => {
                args.push(s[start..idx].trim().to_string());
                start = idx + ch.len_utf8();
                at_value_start = true;
            }
            ',' => at_value_start = true,
            ch if ch.is_whitespace() && at_value_start => {}
            _ => {
                at_value_start = false;
            }
        }
    }

    args.push(s[start..].trim().to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_provider::{ProviderBuilder, mock::Asserter};
    use foundry_common::abi::{encode_function_args, get_func};

    const ADDRESS: &str = "0x1234567890123456789012345678901234567890";

    async fn assert_resolves(cases: &[(&str, &str, &[&str])]) {
        let provider = ProviderBuilder::new().connect_mocked_client(Asserter::new());
        for &(suffix, sig, args) in cases {
            let spec = format!("{ADDRESS}{suffix}");
            let call = CallSpec::parse(&spec)
                .unwrap()
                .resolve(0, Chain::from_id(1), &provider, None, None)
                .await
                .unwrap();
            let expected = encode_function_args(&get_func(sig).unwrap(), args).unwrap();
            assert_eq!(call.input.as_ref(), expected, "call spec: {spec}");
        }
    }

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
            "0x1234567890123456789012345678901234567890::foo((uint256[],string)):([1,2],hello)",
        )
        .unwrap();
        assert_eq!(spec.sig, Some("foo((uint256[],string))".to_string()));
        assert_eq!(spec.args, vec!["([1,2],hello)"]);
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
        assert_eq!(split_call_args("\"a,b\",9"), vec!["\"a,b\"", "9"]);
        assert_eq!(split_call_args("(7,\"a],b\"),9"), vec!["(7,\"a],b\")", "9"]);
        assert_eq!(split_call_args("can't,9"), vec!["can't", "9"]);
    }

    #[tokio::test]
    async fn test_resolve_nested_args() {
        const MIXED_ARGS: &str = "[(0x1111111111111111111111111111111111111111,[1,2]),(0x2222222222222222222222222222222222222222,[3])]";
        let mixed =
            format!("{ADDRESS}:1ether:airdrop((address,uint256[])[],bytes):{MIXED_ARGS},0x00");
        assert_eq!(CallSpec::parse(&mixed).unwrap().value, parse_ether_value("1ether").unwrap());
        assert_resolves(&[
            ("::foo(uint256[][]):[[1,2],[3,4]]", "foo(uint256[][])", &["[[1,2],[3,4]]"]),
            (
                ":1ether:airdrop((address,uint256[])[],bytes):[(0x1111111111111111111111111111111111111111,[1,2]),(0x2222222222222222222222222222222222222222,[3])],0x00",
                "airdrop((address,uint256[])[],bytes)",
                &[MIXED_ARGS, "0x00"],
            ),
            (
                "::foo((uint256,string),uint256):(7,hello),9",
                "foo((uint256,string),uint256)",
                &["(7,hello)", "9"],
            ),
            (
                "::foo((uint256,string),uint256):(7,\"a],b\"),9",
                "foo((uint256,string),uint256)",
                &["(7,\"a],b\")", "9"],
            ),
        ])
        .await;
    }

    #[tokio::test]
    async fn test_resolve_preserves_legacy_string_args() {
        assert_resolves(&[
            ("::foo(string,string):(a,b)", "foo(string,string)", &["(a", "b)"]),
            ("::foo(string,uint256):hello[,9", "foo(string,uint256)", &["hello[", "9"]),
            ("::foo(string[],uint256):[\"[\"],9", "foo(string[],uint256)", &["[\"[\"]", "9"]),
            ("::foo(string,uint256):can't,9", "foo(string,uint256)", &["can't", "9"]),
        ])
        .await;
    }

    #[tokio::test]
    async fn test_resolve_quoted_commas_and_colons() {
        assert_resolves(&[
            ("::foo(string,uint256):\"a,b\",9", "foo(string,uint256)", &["\"a,b\"", "9"]),
            ("::foo(string,uint256):'a,b',9", "foo(string,uint256)", &["'a,b'", "9"]),
            ("::foo(string,uint256): urn:a:b , 9", "foo(string,uint256)", &["urn:a:b", "9"]),
        ])
        .await;
    }

    #[tokio::test]
    async fn test_resolve_empty_args() {
        assert_resolves(&[
            ("::foo()", "foo()", &[]),
            ("::foo(string):", "foo(string)", &[""]),
            ("::foo(string,string):,value", "foo(string,string)", &["", "value"]),
            ("::foo(string,string):value,", "foo(string,string)", &["value", ""]),
            ("::foo(uint256[]):[]", "foo(uint256[])", &["[]"]),
        ])
        .await;
    }

    #[tokio::test]
    async fn test_resolve_rejects_malformed_nested_args() {
        let provider = ProviderBuilder::new().connect_mocked_client(Asserter::new());
        for args in ["[1,2)", "[1,2", "[1,2]]", "\"a,b,9"] {
            let sig = if args.starts_with('[') { "foo(uint256[])" } else { "foo(string,uint256)" };
            let spec = CallSpec::parse(&format!("{ADDRESS}::{sig}:{args}")).unwrap();
            assert!(
                spec.resolve(0, Chain::from_id(1), &provider, None, None).await.is_err(),
                "malformed arguments unexpectedly encoded: {args}"
            );
        }
    }
}
