use alloy_provider::Provider;
use clap::Parser;
use eyre::Result;
use foundry_cli::{json::print_json_value_or_scalar, opts::RpcOpts, utils, utils::LoadConfig};
use itertools::Itertools;

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
            .raw_request::<_, serde_json::Value>(method.into(), params)
            .await?;
        let result = serde_json::to_string(&result)?;
        print_json_value_or_scalar(result)
    }
}

fn value_or_string(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
}
