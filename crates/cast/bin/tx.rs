use alloy_json_abi::Function;
use alloy_network::TransactionBuilder;
use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_transport::Transport;
use eyre::Result;
use foundry_cli::{opts::TransactionOpts, utils::parse_function_args};
use foundry_common::ens::NameOrAddress;
use foundry_config::Chain;

/// Prevents a misconfigured hwlib from sending a transaction that defies user-specified --from
pub fn validate_from_address(
    specified_from: Option<Address>,
    signer_address: Address,
) -> Result<()> {
    if let Some(specified_from) = specified_from {
        if specified_from != signer_address {
            eyre::bail!(
                "\
The specified sender via CLI/env vars does not match the sender configured via
the hardware wallet's HD Path.
Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which
corresponds to the sender, or let foundry automatically detect it by not specifying any sender address."
            )
        }
    }
    Ok(())
}

/// Ensures the transaction is either a contract deployment or a recipient address is specified
pub fn validate_to_address(code: &Option<String>, to: &Option<NameOrAddress>) -> Result<()> {
    if code.is_none() && to.is_none() {
        eyre::bail!("Must specify a recipient address or contract code to deploy");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn build_tx<
    P: Provider<T>,
    T: Transport + Clone,
    F: Into<NameOrAddress>,
    TO: Into<NameOrAddress>,
>(
    provider: &P,
    from: F,
    to: Option<TO>,
    code: Option<String>,
    sig: Option<String>,
    args: Vec<String>,
    tx: TransactionOpts,
    chain: impl Into<Chain>,
    etherscan_api_key: Option<String>,
) -> Result<(TransactionRequest, Option<Function>)> {
    let chain = chain.into();

    let from = from.into().resolve(provider).await?;
    let to = if let Some(to) = to { Some(to.into().resolve(provider).await?) } else { None };

    let mut req = TransactionRequest::default()
        .with_to(to.into())
        .with_from(from)
        .with_value(tx.value.unwrap_or_default())
        .with_chain_id(chain.id());

    req.set_nonce(
        if let Some(nonce) = tx.nonce {
            nonce
        } else {
            provider.get_transaction_count(from, None).await?
        }
        .to(),
    );

    if tx.legacy || chain.is_legacy() {
        req.set_gas_price(if let Some(gas_price) = tx.gas_price {
            gas_price
        } else {
            provider.get_gas_price().await?
        });
    } else {
        let (max_fee, priority_fee) = match (tx.gas_price, tx.priority_gas_price) {
            (Some(gas_price), Some(priority_gas_price)) => (gas_price, priority_gas_price),
            (_, _) => {
                let estimate = provider.estimate_eip1559_fees(None).await?;
                (
                    tx.gas_price.unwrap_or(estimate.max_fee_per_gas),
                    tx.priority_gas_price.unwrap_or(estimate.max_priority_fee_per_gas),
                )
            }
        };

        req.set_max_fee_per_gas(max_fee);
        req.set_max_priority_fee_per_gas(priority_fee);
    }

    let params = sig.as_deref().map(|sig| (sig, args));
    let (data, func) = if let Some(code) = code {
        let mut data = hex::decode(code)?;

        if let Some((sig, args)) = params {
            let (mut sigdata, _) =
                parse_function_args(sig, args, None, chain, provider, etherscan_api_key.as_deref())
                    .await?;
            data.append(&mut sigdata);
        }

        (data, None)
    } else if let Some((sig, args)) = params {
        parse_function_args(sig, args, None, chain, provider, etherscan_api_key.as_deref()).await?
    } else {
        (Vec::new(), None)
    };

    req.set_input(data.into());

    req.set_gas_limit(if let Some(gas_limit) = tx.gas_limit {
        gas_limit
    } else {
        provider.estimate_gas(&req, None).await?
    });

    Ok((req, func))
}
