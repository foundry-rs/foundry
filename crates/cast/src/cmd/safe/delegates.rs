use super::{
    rpc_provider,
    service::{SafeDelegatesResponse, SafeServiceOpts},
    signing::sign_delegate,
};
use alloy_primitives::Address;
use alloy_signer::Signer;
use clap::Args;
use eyre::{Context, Result, ensure};
use foundry_cli::{
    json::{print_json_object, print_scalar},
    opts::RpcOpts,
};
use foundry_wallets::WalletOpts;
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::HashSet;

/// CLI arguments for `cast safe add-delegate`.
#[derive(Args, Debug)]
pub struct AddDelegateArgs {
    /// Safe account address.
    safe: Address,

    /// Address allowed to propose transactions.
    delegate: Address,

    /// Human-readable delegate label.
    #[arg(long)]
    label: String,

    #[command(flatten)]
    service: Box<SafeServiceOpts>,

    #[command(flatten)]
    rpc: Box<RpcOpts>,

    #[command(flatten)]
    wallet: Box<WalletOpts>,
}

impl AddDelegateArgs {
    pub(super) async fn run(self) -> Result<()> {
        let Self { safe, delegate, label, service, rpc, wallet } = self;
        ensure!(!label.trim().is_empty(), "delegate label cannot be empty");
        let body = json!({ "delegate": delegate.to_checksum(None), "label": label });
        submit_delegate(
            &service,
            &rpc,
            &wallet,
            safe,
            delegate,
            Method::POST,
            "v2/delegates/",
            body,
        )
        .await
    }
}

/// CLI arguments for `cast safe list-delegates`.
#[derive(Args, Debug)]
pub struct ListDelegatesArgs {
    /// Safe account address.
    safe: Address,

    #[command(flatten)]
    service: Box<SafeServiceOpts>,

    #[command(flatten)]
    rpc: Box<RpcOpts>,
}

impl ListDelegatesArgs {
    pub(super) async fn run(self) -> Result<()> {
        let Self { safe, service, rpc } = self;
        let chain_id = if service.service_url.is_some() { 0 } else { rpc_provider(&rpc).await?.1 };
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
        print_json_object(delegates)
    }
}

/// CLI arguments for `cast safe remove-delegate`.
#[derive(Args, Debug)]
pub struct RemoveDelegateArgs {
    /// Safe account address.
    safe: Address,

    /// Delegate address to remove.
    delegate: Address,

    #[command(flatten)]
    service: Box<SafeServiceOpts>,

    #[command(flatten)]
    rpc: Box<RpcOpts>,

    #[command(flatten)]
    wallet: Box<WalletOpts>,
}

impl RemoveDelegateArgs {
    pub(super) async fn run(self) -> Result<()> {
        let Self { safe, delegate, service, rpc, wallet } = self;
        let path = format!("v2/delegates/{}/", delegate.to_checksum(None));
        submit_delegate(&service, &rpc, &wallet, safe, delegate, Method::DELETE, &path, json!({}))
            .await
    }
}

/// Signs the delegate TOTP message with the wallet and submits `body` (extended with the Safe,
/// delegator and signature) to the Transaction Service, printing the delegate on success.
#[allow(clippy::too_many_arguments)]
async fn submit_delegate(
    service: &SafeServiceOpts,
    rpc: &RpcOpts,
    wallet: &WalletOpts,
    safe: Address,
    delegate: Address,
    method: Method,
    path: &str,
    mut body: Value,
) -> Result<()> {
    let (_, chain_id) = rpc_provider(rpc).await?;
    let signer = wallet.signer().await?;
    let signature = sign_delegate(&signer, delegate, chain_id).await?;
    body["safe"] = safe.to_checksum(None).into();
    body["delegator"] = signer.address().to_checksum(None).into();
    body["signature"] = signature.into();
    let url = service.endpoint(chain_id, path)?;
    service.empty_response(service.request(method, url).json(&body)).await?;
    print_scalar(delegate.to_checksum(None))
}
