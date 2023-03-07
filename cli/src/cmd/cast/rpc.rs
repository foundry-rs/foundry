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
    #[clap(
        short = 'w',
        long,
        help = r#"Pass the "params" as is"#,
        long_help = r#"Pass the "params" as is

If --raw is passed the first PARAM will be taken as the value of "params". If no params are given, stdin will be used. For example:

rpc eth_getBlockByNumber '["0x123", false]' --raw
    => {"method": "eth_getBlockByNumber", "params": ["0x123", false] ... }"#
    )]
    raw: bool,
    #[clap(value_name = "METHOD", help = "RPC method name")]
    method: String,
    #[clap(
        value_name = "PARAMS",
        help = "RPC parameters",
        long_help = r#"RPC parameters

Parameters are interpreted as JSON and then fall back to string. For example:

rpc eth_getBlockByNumber 0x123 false
    => {"method": "eth_getBlockByNumber", "params": ["0x123", false] ... }"#
    )]
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
                Self::to_json_or_string(params.into_iter().join(" "))
            }
        } else {
            serde_json::Value::Array(params.into_iter().map(Self::to_json_or_string).collect())
        };
        println!("{}", Cast::new(provider).rpc(&method, params).await?);
        Ok(())
    }
    fn to_json_or_string(value: String) -> serde_json::Value {
        serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
    }
}
