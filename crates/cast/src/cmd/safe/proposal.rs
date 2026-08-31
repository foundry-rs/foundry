use super::{
    SafeOperation,
    contracts::ISafe,
    service::{SafeServiceOpts, SafeTransaction},
    signing::sign_safe_hash,
};
use alloy_network::Ethereum;
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_provider::Provider;
use alloy_signer::Signer;
use eyre::Result;
use foundry_cli::{json::print_scalar, opts::RpcOpts, utils::LoadConfig};
use foundry_common::{
    abi::{encode_function_args, get_func},
    provider::ProviderBuilder,
};
use foundry_wallets::WalletOpts;
use reqwest::Method;
use serde_json::json;

#[allow(clippy::too_many_arguments)]
pub(super) async fn propose(
    safe: Address,
    to: Address,
    sig: Option<String>,
    args: Vec<String>,
    data: Option<Bytes>,
    value: U256,
    operation: SafeOperation,
    safe_tx_gas: U256,
    base_gas: U256,
    gas_price: U256,
    gas_token: Address,
    refund_receiver: Address,
    nonce: Option<U256>,
    origin: Option<String>,
    service: SafeServiceOpts,
    rpc: RpcOpts,
    wallet: WalletOpts,
) -> Result<()> {
    let data = match (data, sig) {
        (Some(data), None) => data,
        (None, Some(sig)) => encode_function_args(&get_func(&sig)?, &args)?.into(),
        (None, None) => Bytes::new(),
        (Some(_), Some(_)) => unreachable!("enforced by clap"),
    };
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain_id = provider.get_chain_id().await?;
    let onchain_nonce = ISafe::new(safe, &provider).nonce().call().await?;
    let nonce = match nonce {
        Some(nonce) => nonce,
        None => service.next_nonce(chain_id, safe, onchain_nonce).await?,
    };
    let mut transaction = SafeTransaction {
        safe,
        to,
        value: value.to_string(),
        data,
        operation: operation as u8,
        safe_tx_gas: safe_tx_gas.to_string(),
        base_gas: base_gas.to_string(),
        gas_price: gas_price.to_string(),
        gas_token,
        refund_receiver,
        nonce: nonce.to_string(),
        safe_tx_hash: B256::ZERO,
        confirmations: Vec::new(),
        is_executed: false,
        transaction_hash: None,
    };
    transaction.safe_tx_hash = transaction.calculate_hash(&provider).await?;
    transaction.show_transaction_summary()?;
    let signer = wallet.signer().await?;
    let signature = sign_safe_hash(&signer, transaction.safe_tx_hash).await?;
    let url = service.endpoint(
        chain_id,
        &format!("v2/safes/{}/multisig-transactions/", transaction.safe.to_checksum(None)),
    )?;
    let body = transaction.proposal_body(signer.address(), signature, origin);
    service.empty_response(service.request(Method::POST, url).json(&body)).await?;
    print_scalar(transaction.safe_tx_hash)?;
    Ok(())
}

pub(super) async fn sign(
    safe: Address,
    safe_tx_hash: B256,
    service: SafeServiceOpts,
    rpc: RpcOpts,
    wallet: WalletOpts,
) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain_id = provider.get_chain_id().await?;
    let transaction = service.get_transaction(chain_id, "v1", safe_tx_hash).await?;
    transaction.verify_hash(safe, &provider).await?;
    transaction.show_transaction_summary()?;
    let signer = wallet.signer().await?;
    let signature = sign_safe_hash(&signer, safe_tx_hash).await?;
    let url = service
        .endpoint(chain_id, &format!("v1/multisig-transactions/{safe_tx_hash}/confirmations/"))?;
    service
        .empty_response(service.request(Method::POST, url).json(&json!({
            "signature": signature,
        })))
        .await?;
    print_scalar(signature)?;
    Ok(())
}
