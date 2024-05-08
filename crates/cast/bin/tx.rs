use alloy_consensus::{SidecarBuilder, SimpleCoder};
use alloy_json_abi::Function;
use alloy_network::{AnyNetwork, TransactionBuilder};
use alloy_primitives::{Address, Bytes, U256};
use alloy_provider::Provider;
use alloy_rpc_types::{BlockId, TransactionRequest, WithOtherFields};
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
    P: Provider<T, AnyNetwork>,
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
    blob_data: Option<Vec<u8>>,
) -> Result<(WithOtherFields<TransactionRequest>, Option<Function>)> {
    let chain = chain.into();

    let from = from.into().resolve(provider).await?;

    let to: Option<Address> =
        if let Some(to) = to { Some(to.into().resolve(provider).await?) } else { None };

    let sidecar = blob_data
        .map(|data| {
            let mut coder = SidecarBuilder::<SimpleCoder>::default();
            coder.ingest(&data);
            coder.build()
        })
        .transpose()?;

    let mut req = WithOtherFields::new(TransactionRequest::default())
        .with_to(to.unwrap_or_default())
        .with_from(from)
        .with_value(tx.value.unwrap_or_default())
        .with_chain_id(chain.id());

    if let Some(sidecar) = sidecar {
        req.set_blob_sidecar(sidecar);
        req.populate_blob_hashes();
        req.set_max_fee_per_blob_gas(
            // If blob_base_fee is 0, uses 1 wei as minimum.
            tx.blob_gas_price.map_or(provider.get_blob_base_fee().await?.max(1), |g| g.to()),
        );
    }

    req.set_nonce(if let Some(nonce) = tx.nonce {
        nonce.to()
    } else {
        provider.get_transaction_count(from, BlockId::latest()).await?
    });

    if tx.legacy || chain.is_legacy() {
        req.set_gas_price(if let Some(gas_price) = tx.gas_price {
            gas_price.to()
        } else {
            provider.get_gas_price().await?
        });
    } else {
        let (max_fee, priority_fee) = match (tx.gas_price, tx.priority_gas_price) {
            (Some(gas_price), Some(priority_gas_price)) => (gas_price, priority_gas_price),
            (_, _) => {
                let estimate = provider.estimate_eip1559_fees(None).await?;
                (
                    tx.gas_price.unwrap_or(U256::from(estimate.max_fee_per_gas)),
                    tx.priority_gas_price.unwrap_or(U256::from(estimate.max_priority_fee_per_gas)),
                )
            }
        };

        req.set_max_fee_per_gas(max_fee.to());
        req.set_max_priority_fee_per_gas(priority_fee.to());
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

    req.set_input::<Bytes>(data.into());

    req.set_gas_limit(if let Some(gas_limit) = tx.gas_limit {
        gas_limit.to()
    } else {
        provider.estimate_gas(&req, BlockId::latest()).await?
    });

    Ok((req, func))
}
