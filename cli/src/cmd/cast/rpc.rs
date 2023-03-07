use crate::utils::try_consume_config_rpc_url;
use cast::Cast;
use clap::Parser;
use eyre::Result;
use foundry_common::try_get_http_provider;
use itertools::Itertools;

/// CLI arguments for `cast rpc`.
#[derive(Debug, Clone, Parser)]
pub struct RpcArgs {
    #[clap(short, long, env = "ETH_RPC_URL", value_name = "URL")]
    rpc_url: Option<String>,

    /// Send raw JSON parameters
    #[clap(
        short = 'w',
        long,
        long_help = r#"The first param will be interpreted as a raw JSON array of params.
If no params are given, stdin will be used. For example:

cast rpc eth_getBlockByNumber '["0x123", false]' --raw
    => {"method": "eth_getBlockByNumber", "params": ["0x123", false] ... }"#
    )]
    raw: bool,

    /// RPC method name
    method: String,

    /// RPC parameters
    #[clap(long_help = r#"RPC parameters interpreted as JSON:

cast rpc eth_getBlockByNumber 0x123 false
    => {"method": "eth_getBlockByNumber", "params": ["0x123", false] ... }"#)]
    params: Vec<String>,
}

impl RpcArgs {
    pub async fn run(self) -> Result<()> {
        let RpcArgs { rpc_url, raw, method, params } = self;

        let rpc_url = try_consume_config_rpc_url(rpc_url)?;
        let provider = try_get_http_provider(rpc_url)?;
        let params = if raw {
            if params.is_empty() {
                serde_json::Deserializer::from_reader(std::io::stdin())
                    .into_iter()
                    .next()
                    .transpose()?
                    .ok_or_else(|| eyre::format_err!("Empty JSON parameters"))?
            } else {
                value_or_string(params.into_iter().join(" "))
            }
        } else {
            serde_json::Value::Array(params.into_iter().map(value_or_string).collect())
        };
        println!("{}", Cast::new(provider).rpc(&method, params).await?);
        Ok(())
    }
}

fn value_or_string(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
}
