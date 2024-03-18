use alloy_primitives::Address;
use alloy_providers::tmp::TempProvider;
use cast::{TxBuilder, TxBuilderOutput};
use eyre::Result;
use foundry_cli::opts::TransactionOpts;
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
pub async fn build_tx<P: TempProvider, F: Into<NameOrAddress>, T: Into<NameOrAddress>>(
    provider: &P,
    from: F,
    to: Option<T>,
    code: Option<String>,
    sig: Option<String>,
    args: Vec<String>,
    tx: TransactionOpts,
    chain: impl Into<Chain>,
    etherscan_api_key: Option<String>,
) -> Result<TxBuilderOutput> {
    let from = from.into().resolve(provider).await?;
    let to = if let Some(to) = to { Some(to.into().resolve(provider).await?) } else { None };

    let mut builder = TxBuilder::new(provider, from, to, chain, tx.legacy).await?;
    builder
        .etherscan_api_key(etherscan_api_key)
        .gas(tx.gas_limit)
        .gas_price(tx.gas_price)
        .priority_gas_price(tx.priority_gas_price)
        .value(tx.value)
        .nonce(tx.nonce);

    let params = sig.as_deref().map(|sig| (sig, args));
    if let Some(code) = code {
        let mut data = hex::decode(code)?;

        if let Some((sig, args)) = params {
            let (mut sigdata, _) = builder.create_args(sig, args).await?;
            data.append(&mut sigdata);
        }

        builder.set_data(data);
    } else {
        builder.args(params).await?;
    }

    let builder_output = builder.build();
    Ok(builder_output)
}
