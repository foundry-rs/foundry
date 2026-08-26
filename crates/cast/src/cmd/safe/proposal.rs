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
use eyre::{Result, ensure};
use foundry_cli::{opts::RpcOpts, utils::LoadConfig};
use foundry_common::{
    abi::{encode_function_args, get_func},
    provider::ProviderBuilder,
    sh_println,
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
        safe: safe.to_checksum(None),
        to: to.to_checksum(None),
        value: value.to_string(),
        data,
        operation: operation.as_u8(),
        safe_tx_gas: safe_tx_gas.to_string(),
        base_gas: base_gas.to_string(),
        gas_price: gas_price.to_string(),
        gas_token: gas_token.to_checksum(None),
        refund_receiver: refund_receiver.to_checksum(None),
        nonce: nonce.to_string(),
        safe_tx_hash: B256::ZERO,
        signatures: Bytes::new(),
        confirmations: Vec::new(),
        is_executed: false,
        transaction_hash: None,
    };
    transaction.safe_tx_hash = transaction.calculate_hash(&provider).await?;
    transaction.show_signing_summary()?;
    let signer = wallet.signer().await?;
    let signature = sign_safe_hash(&signer, transaction.safe_tx_hash).await?;
    let url = service.endpoint(
        chain_id,
        &format!("v2/safes/{}/multisig-transactions/", transaction.safe()?.to_checksum(None)),
    )?;
    let body = transaction.proposal_body(signer.address(), signature, origin);
    service.empty_response(service.request(Method::POST, url).json(&body)).await?;
    sh_println!("{}", transaction.safe_tx_hash)?;
    Ok(())
}

pub(super) async fn sign(
    safe_tx_hash: B256,
    service: SafeServiceOpts,
    rpc: RpcOpts,
    wallet: WalletOpts,
) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain_id = provider.get_chain_id().await?;
    let url = service.endpoint(chain_id, &format!("v1/multisig-transactions/{safe_tx_hash}/"))?;
    let transaction: SafeTransaction = service.response(service.request(Method::GET, url)).await?;
    ensure!(
        transaction.safe_tx_hash == safe_tx_hash,
        "Transaction Service returned a different Safe transaction hash"
    );
    transaction.verify_hash(&provider).await?;
    transaction.show_signing_summary()?;
    let signer = wallet.signer().await?;
    let signature = sign_safe_hash(&signer, safe_tx_hash).await?;
    let url = service
        .endpoint(chain_id, &format!("v1/multisig-transactions/{safe_tx_hash}/confirmations/"))?;
    service
        .empty_response(service.request(Method::POST, url).json(&json!({
            "signature": signature,
        })))
        .await?;
    sh_println!("{signature}")?;
    Ok(())
}
