use alloy_chains::Chain;
use alloy_ens::NameOrAddress;
use alloy_json_abi::Function;
use alloy_primitives::{Address, hex};
use alloy_provider::{Provider, network::AnyNetwork};
use eyre::{OptionExt, Result};
use foundry_common::abi::{
    encode_function_args, encode_function_args_raw, get_func, get_func_etherscan,
};
use futures::future::join_all;

async fn resolve_name_args<P: Provider<AnyNetwork>>(args: &[String], provider: &P) -> Vec<String> {
    join_all(args.iter().map(|arg| async {
        if arg.contains('.') {
            let addr = NameOrAddress::Name(arg.to_string()).resolve(provider).await;
            match addr {
                Ok(addr) => addr.to_string(),
                Err(_) => arg.to_string(),
            }
        } else {
            arg.to_string()
        }
    }))
    .await
}

pub async fn parse_function_args<P: Provider<AnyNetwork>>(
    sig: &str,
    args: Vec<String>,
    to: Option<Address>,
    chain: Chain,
    provider: &P,
    etherscan_api_key: Option<&str>,
) -> Result<(Vec<u8>, Option<Function>)> {
    if sig.trim().is_empty() {
        eyre::bail!("Function signature or calldata must be provided.")
    }

    let args = resolve_name_args(&args, provider).await;

    if let Ok(data) = hex::decode(sig) {
        return Ok((data, None));
    }

    let func = if sig.contains('(') {
        get_func(sig)?
    } else if sig.starts_with("0x") || sig.starts_with("0X") {
        eyre::bail!(
            "Invalid hex calldata: '{}'. Hex strings must have an even number of digits (e.g., use '0x00' instead of '0x0').",
            sig
        );
    } else {
        match etherscan_api_key {
            Some(key) => {
                info!(
                    "function signature does not contain parentheses, fetching function data from Etherscan"
                );
                let to =
                    to.ok_or_eyre("A 'to' address must be provided to fetch function data.")?;
                get_func_etherscan(sig, to, &args, chain, key).await?
            }
            None => get_func("fallback()")?,
        }
    };

    if to.is_none() {
        // if this is a CREATE call we must exclude the (constructor) function selector: https://github.com/foundry-rs/foundry/issues/10947
        Ok((encode_function_args_raw(&func, &args)?, Some(func)))
    } else {
        Ok((encode_function_args(&func, &args)?, Some(func)))
    }
}
