use alloy_provider::Provider;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::{print_json_success, print_json_value_or_scalar},
    opts::RpcOpts,
    utils,
    utils::LoadConfig,
};
use foundry_common::shell;
use itertools::Itertools;
use serde_json::value::RawValue;

/// CLI arguments for `cast rpc`.
#[derive(Clone, Debug, Parser)]
pub struct RpcArgs {
    /// RPC method name
    method: String,

    /// RPC parameters
    ///
    /// Interpreted as JSON:
    ///
    /// cast rpc eth_getBlockByNumber 0x123 false
    /// => {"method": "eth_getBlockByNumber", "params": ["0x123", false] ... }
    params: Vec<String>,

    /// Send raw JSON parameters
    ///
    /// The first param will be interpreted as a raw JSON array of params.
    /// If no params are given, stdin will be used. For example:
    ///
    /// cast rpc eth_getBlockByNumber '["0x123", false]' --raw
    ///     => {"method": "eth_getBlockByNumber", "params": ["0x123", false] ... }
    #[arg(long, short = 'w')]
    raw: bool,

    #[command(flatten)]
    rpc: RpcOpts,
}

impl RpcArgs {
    pub async fn run(self) -> Result<()> {
        let Self { raw, method, params, rpc } = self;
        let config = rpc.load_config()?;

        let params = if !raw {
            serde_json::Value::Array(params.into_iter().map(value_or_string).collect())
        } else if params.is_empty() {
            serde_json::Deserializer::from_reader(std::io::stdin())
                .into_iter()
                .next()
                .transpose()?
                .ok_or_else(|| eyre::format_err!("Empty JSON parameters"))?
        } else {
            value_or_string(params.into_iter().join(" "))
        };

        let result = utils::get_provider(&config)?
            .raw_request::<_, Box<RawValue>>(method.into(), params)
            .await?;

        // Print the result as the node sent it if that is already the compact form, which
        // avoids a `Value` tree of large results. Other results go through a `Value`.
        if !is_compact_json(result.get()) {
            let result = serde_json::from_str::<serde_json::Value>(result.get())?;
            return print_json_value_or_scalar(result.to_string());
        }
        if shell::is_json() {
            print_json_success(&*result)
        } else {
            sh_println!("{}", result.get())?;
            Ok(())
        }
    }
}

fn value_or_string(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
}

/// Returns `true` if `json` is valid JSON text that a `serde_json::Value` serializes back
/// unchanged: no whitespace between tokens, and only the string escapes that `serde_json` writes.
/// Duplicate object keys are not detected.
fn is_compact_json(json: &str) -> bool {
    let mut bytes = json.bytes();
    let mut in_string = false;
    while let Some(byte) = bytes.next() {
        match (in_string, byte) {
            (false, b' ' | b'\t' | b'\n' | b'\r') => return false,
            (_, b'"') => in_string = !in_string,
            (true, b'\\') => match bytes.next() {
                Some(b'"' | b'\\' | b'b' | b'f' | b'n' | b'r' | b't') => {}
                // Control characters without a short escape, as lowercase `\u00xx`.
                Some(b'u') => {
                    let hex = [(); 4].map(|()| bytes.next().unwrap_or_default());
                    if !matches!(hex, [b'0', b'0', b'0' | b'1', b'0'..=b'9' | b'a'..=b'f'])
                        || matches!(&hex[2..], b"08" | b"09" | b"0a" | b"0c" | b"0d")
                    {
                        return false;
                    }
                }
                _ => return false,
            },
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_json_matches_serde_output() {
        for json in [
            r#"{"number":"0x123","logs":[{"data":"0x01"}]}"#,
            r#"{"n":123456789012345678901234567890,"f":1.50}"#,
            r#"{"s":"a \"q\" \\ \n \u001f é <>&"}"#,
            r#""0x1""#,
        ] {
            assert!(is_compact_json(json), "{json}");
            let value = serde_json::from_str::<serde_json::Value>(json).unwrap();
            assert_eq!(value.to_string(), json);
        }
        for json in [
            r#"{"a": 1}"#,
            "{\n\"a\":1}",
            r#"{"s":"\u003c"}"#,
            r#"{"s":"\/"}"#,
            r#"{"s":"\u000a"}"#,
            r#"{"s":"\u001F"}"#,
        ] {
            assert!(!is_compact_json(json), "{json}");
        }
    }
}
