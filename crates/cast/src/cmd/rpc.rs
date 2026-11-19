use alloy_provider::Provider;
use clap::Parser;
use eyre::{Result, WrapErr};
use foundry_cli::{
    json::{print_json_success, print_json_value_or_scalar},
    opts::RpcOpts,
    utils,
    utils::LoadConfig,
};
use foundry_common::shell;
use serde_json::{Value, value::RawValue};

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
            // Arguments that are not valid JSON, such as `0x123`, are passed as strings.
            Value::Array(
                params
                    .into_iter()
                    .map(|param| serde_json::from_str(&param).unwrap_or(Value::String(param)))
                    .collect(),
            )
        } else if params.is_empty() {
            serde_json::Deserializer::from_reader(std::io::stdin())
                .into_iter()
                .next()
                .transpose()?
                .ok_or_else(|| eyre::format_err!("Empty JSON parameters"))?
        } else {
            serde_json::from_str(&params.join(" ")).wrap_err("invalid raw JSON parameters")?
        };

        let result = utils::get_provider(&config)?
            .raw_request::<_, Box<RawValue>>(method.into(), params)
            .await?;

        // Print the result as the node sent it, which avoids a `Value` tree of large results.
        // Without whitespace or escapes, a `Value` would serialize it back to the same text.
        if result.get().contains([' ', '\t', '\n', '\r', '\\']) {
            let result = serde_json::from_str::<Value>(result.get())?;
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
