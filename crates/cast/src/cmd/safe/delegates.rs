use super::{
    service::{SafeDelegatesResponse, SafeServiceOpts},
    signing::sign_delegate,
};
use alloy_network::Ethereum;
use alloy_primitives::Address;
use alloy_provider::Provider;
use alloy_signer::Signer;
use eyre::{Context, Result, ensure};
use foundry_cli::{
    json::{print_json_object, print_scalar},
    opts::RpcOpts,
    utils::LoadConfig,
};
use foundry_common::provider::ProviderBuilder;
use foundry_wallets::WalletOpts;
use reqwest::Method;
use serde_json::json;
use std::collections::HashSet;

pub(super) async fn add(
    safe: Address,
    delegate: Address,
    label: String,
    service: SafeServiceOpts,
    rpc: RpcOpts,
    wallet: WalletOpts,
) -> Result<()> {
    ensure!(!label.trim().is_empty(), "delegate label cannot be empty");
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain_id = provider.get_chain_id().await?;
    let signer = wallet.signer().await?;
    let signature = sign_delegate(&signer, delegate, chain_id).await?;
    let url = service.endpoint(chain_id, "v2/delegates/")?;
    let body = json!({
        "safe": safe.to_checksum(None),
        "delegate": delegate.to_checksum(None),
        "delegator": signer.address().to_checksum(None),
        "label": label,
        "signature": signature,
    });
    service.empty_response(service.request(Method::POST, url).json(&body)).await?;
    print_scalar(delegate.to_checksum(None))?;
    Ok(())
}

pub(super) async fn list(safe: Address, service: SafeServiceOpts, rpc: RpcOpts) -> Result<()> {
    let chain_id = if service.service_url.is_some() {
        0
    } else {
        let config = rpc.load_config()?;
        ProviderBuilder::<Ethereum>::from_config(&config)?.build()?.get_chain_id().await?
    };
    let mut url = service.endpoint(chain_id, "v2/delegates/")?;
    url.query_pairs_mut().append_pair("safe", &safe.to_checksum(None));
    let origin = url.origin();
    let path = url.path().to_string();
    let mut visited = HashSet::new();
    let mut delegates = Vec::new();
    loop {
        ensure!(visited.insert(url.clone()), "delegate pagination contains a cycle at {url}");
        let mut response: SafeDelegatesResponse =
            service.response(service.request(Method::GET, url.clone())).await?;
        delegates.append(&mut response.results);
        let Some(next) = response.next else { break };
        let next = url.join(&next).wrap_err("invalid delegate pagination URL")?;
        ensure!(
            next.origin() == origin && next.path() == path,
            "delegate pagination URL points outside the Transaction Service endpoint: {next}"
        );
        url = next;
    }
    print_json_object(delegates)?;
    Ok(())
}

pub(super) async fn remove(
    safe: Address,
    delegate: Address,
    service: SafeServiceOpts,
    rpc: RpcOpts,
    wallet: WalletOpts,
) -> Result<()> {
    let config = rpc.load_config()?;
    let provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain_id = provider.get_chain_id().await?;
    let signer = wallet.signer().await?;
    let signature = sign_delegate(&signer, delegate, chain_id).await?;
    let url =
        service.endpoint(chain_id, &format!("v2/delegates/{}/", delegate.to_checksum(None)))?;
    let body = json!({
        "safe": safe.to_checksum(None),
        "delegator": signer.address().to_checksum(None),
        "signature": signature,
    });
    service.empty_response(service.request(Method::DELETE, url).json(&body)).await?;
    print_scalar(delegate.to_checksum(None))?;
    Ok(())
}
