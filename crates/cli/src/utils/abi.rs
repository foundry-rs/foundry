use alloy_chains::Chain;
use alloy_ens::NameOrAddress;
use alloy_json_abi::Function;
use alloy_primitives::{Address, hex};
use alloy_provider::{Network, Provider};
use eyre::{OptionExt, Result};
use foundry_common::abi::{
    encode_function_args, encode_function_args_raw, get_func, get_funcs_etherscan,
};
use futures::future::join_all;

pub async fn parse_function_args<N: Network, P: Provider<N>>(
    sig: &str,
    args: Vec<String>,
    to: Option<Address>,
    chain: Chain,
    provider: &P,
    etherscan_api_key: Option<&str>,
    etherscan_api_url: Option<&str>,
) -> Result<(Vec<u8>, Option<Function>)> {
    if sig.trim().is_empty() {
        eyre::bail!("Function signature or calldata must be provided.");
    }

    // Try to decode as hex calldata first, otherwise treat as function signature
    if let Ok(data) = hex::decode(sig) {
        return Ok((data, None));
    } else if sig.starts_with("0x") || sig.starts_with("0X") {
        let e = hex::decode(sig).unwrap_err();
        eyre::bail!("Invalid hex calldata '{}': {e}", sig);
    }

    let (func, args) = if sig.contains('(') {
        // a regular function signature with parentheses
        let func = get_func(sig)?;
        let args = resolve_name_args(&func, &args, provider).await;
        (func, args)
    } else {
        info!(
            "function signature does not contain parentheses, fetching function data from Etherscan"
        );
        let etherscan_api_key = etherscan_api_key.ok_or_eyre(
            "Function signature does not contain parentheses. If you wish to fetch function data from Etherscan, please provide an API key.",
        )?;
        let to = to.ok_or_eyre("A 'to' address must be provided to fetch function data.")?;
        let funcs =
            get_funcs_etherscan(sig, to, chain, etherscan_api_key, etherscan_api_url).await?;
        let mut found = None;
        for func in funcs {
            let resolved = resolve_name_args(&func, &args, provider).await;
            if encode_function_args(&func, &resolved).is_ok() {
                found = Some((func, resolved));
                break;
            }
        }
        found.ok_or_eyre("Function not found in abi")?
    };

    if to.is_none() {
        // if this is a CREATE call we must exclude the (constructor) function selector: https://github.com/foundry-rs/foundry/issues/10947
        Ok((encode_function_args_raw(&func, &args)?, Some(func)))
    } else {
        Ok((encode_function_args(&func, &args)?, Some(func)))
    }
}

/// Resolves ENS names passed to `address` parameters. Other arguments are kept as typed, so values
/// such as `1.5ether` or `alice.eth` passed to a `uint256` or `string` are never sent to ENS.
async fn resolve_name_args<N: Network, P: Provider<N>>(
    func: &Function,
    args: &[String],
    provider: &P,
) -> Vec<String> {
    join_all(args.iter().enumerate().map(|(i, arg)| async move {
        let is_address = func.inputs.get(i).is_some_and(|param| param.ty == "address");
        if is_address && arg.contains('.') {
            match NameOrAddress::Name(arg.clone()).resolve(provider).await {
                Ok(addr) => addr.to_string(),
                Err(_) => arg.clone(),
            }
        } else {
            arg.clone()
        }
    }))
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_network::AnyNetwork;
    use alloy_primitives::{Bytes, U256, address};
    use alloy_provider::ProviderBuilder;
    use alloy_transport::mock::Asserter;

    #[tokio::test]
    async fn resolves_ens_names_only_for_address_params() {
        let attacker = address!("0x0000000000000000000000000000000000895440");
        // Answers every ENS lookup with `attacker`, as a malicious RPC would.
        let asserter = Asserter::new();
        let lookup = DynSolValue::Tuple(vec![
            DynSolValue::Bytes(DynSolValue::Address(attacker).abi_encode()),
            DynSolValue::Address(Address::ZERO),
        ])
        .abi_encode_params();
        for _ in 0..3 {
            asserter.push_success(&Bytes::from(lookup.clone()));
        }
        let provider =
            ProviderBuilder::<_, _, AnyNetwork>::default().connect_mocked_client(asserter);

        let args = vec!["alice.eth".to_string(), "1.5e6".to_string(), "bob.eth".to_string()];
        let (data, _) = parse_function_args(
            "f(address,uint256,string)",
            args,
            Some(Address::ZERO),
            Chain::mainnet(),
            &provider,
            None,
            None,
        )
        .await
        .unwrap();

        let ty =
            DynSolType::Tuple(vec![DynSolType::Address, DynSolType::Uint(256), DynSolType::String]);
        assert_eq!(
            ty.abi_decode_params(&data[4..]).unwrap(),
            DynSolValue::Tuple(vec![
                DynSolValue::Address(attacker),
                DynSolValue::Uint(U256::from(1_500_000), 256),
                DynSolValue::String("bob.eth".to_string()),
            ])
        );
    }
}
