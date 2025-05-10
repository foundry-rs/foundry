use crate::abi::{
    EnsRegistry, NameWrapper, NameWrapper::isWrappedCall, PublicResolver, PublicResolver::addrCall,
    ReverseRegistrar,
};
use alloy_provider::{
    network::{AnyNetwork, EthereumWallet, NetworkWallet},
    Provider, ProviderBuilder, WalletProvider,
};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{
    private::{keccak256, Address, B256},
    SolCall,
};
use clap::Parser;
use eyre::Result;
use foundry_cli::{utils, utils::LoadConfig};
use foundry_common::ens::namehash;
use foundry_wallets::WalletSigner;
use serde::{Deserialize, Serialize};
use std::{
    any::Any,
    io::{stdout, Write},
    sync::Arc,
};

// todo abhi: change this to actual url after api deployed
pub static CONFIG_API_URL: &str = "http://localhost:3001/config";
pub static METRICS_API_URL: &str = "http://localhost:3001/metrics";

#[derive(Debug, Deserialize)]
pub struct ChainConfigResponse {
    reverse_registrar_addr: String,
    ens_registry_addr: String,
    public_resolver_addr: String,
    name_wrapper_addr: String,
}

#[derive(Clone, Debug, Parser)]
pub struct NamingArgs {
    /// The name to set.
    #[arg(long)]
    pub ens_name: String,

    // #[arg(long)]
    // pub auto_name: bool,
    /// The address of the contract.
    #[arg(skip)]
    pub contract_address: Address,

    /// Whether the contract is ReverseClaimable or not.
    #[arg(long, requires = "ens_name")]
    pub reverse_claimer: bool,

    #[arg(skip)]
    pub secret_key: String,
}

#[derive(Debug, Serialize)]
struct Metric {
    contract_address: String,
    ens_name: String,
    deployer_address: String,
    network: u32,
    created_at: u64,
    source: String,
    op_type: String,
}

impl NamingArgs {
    pub async fn run(self) -> Result<()> {
        // set_primary_name(
        //     self.secret_key,
        //     self.contract_address,
        //     self.ens_name,
        //     self.reverse_claimer,
        // )
        // .await
        Ok(())
    }
}

pub async fn set_primary_name<P: Provider<AnyNetwork>>(
    // signer: WalletSigner,
    // key: String,
    provider: P,
    sender_addr: Address,
    contract_addr: Address,
    name: String,
    is_reverse_claimer: bool,
) -> Result<()> {
    // let signer = key.parse::<PrivateKeySigner>()?;
    // todo abhi: pass in a provider
    // let provider: Arc<P> = Arc::new(
    //     ProviderBuilder::<_, _, AnyNetwork>::default()
    //         .with_recommended_fillers()
    //         .wallet(EthereumWallet::new(signer))
    //         .on_provider()
    //         // .connect("https://sepolia.drpc.org")
    //         .await?,
    // );

    let config = get_config(provider.get_chain_id().await?).await?;

    // let sender_addr = provider.default_signer_address();
    // .on_provider(provider);
    // .connect_http(rpc_url.parse()?);

    let reverse_registrar_addr: Address = config.reverse_registrar_addr.parse()?;
    let ens_registry_addr: Address = config.ens_registry_addr.parse()?;
    let public_resolver_addr: Address = config.public_resolver_addr.parse()?;
    let name_wrapper_addr: Address = config.name_wrapper_addr.parse()?;

    // let provider = Arc::new(provider);

    let name_splits = name.split('.').collect::<Vec<&str>>();
    let label = name_splits[0];
    let parent = name_splits[1];
    let tld = name_splits[2];

    // todo abhi: the printlns should be removed
    println!("label: {:?}", label);
    let parent_name = format!("{}.{}", parent, tld);
    println!("parent name: {:?}", parent_name);
    let parent_name_hash = namehash(&parent_name);
    println!("parent name hash: {:?}", parent_name_hash);
    let label_hash = keccak256(&label);
    let complete_name_hash = namehash(&name);
    println!("sender addr: {:?}", sender_addr);

    create_subname(
        sender_addr,
        &provider,
        ens_registry_addr,
        public_resolver_addr,
        name_wrapper_addr,
        label,
        parent_name_hash,
        label_hash,
    )
    .await?;

    set_resolutions(
        &provider,
        public_resolver_addr,
        complete_name_hash,
        name.clone(),
        contract_addr,
        is_reverse_claimer,
        sender_addr,
        reverse_registrar_addr,
    )
    .await?;

    // todo abhi: uncomment this
    // record_metric(Metric {
    //     contract_address: contract_addr.to_string(),
    //     ens_name: name,
    //     deployer_address: sender_addr.to_string(),
    //     network: provider.get_chain_id().await?.try_into()?,
    //     created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
    //     source: "forge".to_string(),
    //     op_type: "set_primary_name".to_string(),
    // })
    // .await;

    Ok(())
}

async fn create_subname<P: Provider<AnyNetwork>>(
    sender_addr: Address,
    provider: &P,
    ens_registry_addr: Address,
    public_resolver_addr: Address,
    name_wrapper_addr: Address,
    label: &str,
    parent_name_hash: B256,
    label_hash: B256,
) -> Result<()> {
    // check if parent domain (e.g. abhi.eth) is wrapped or unwrapped
    let name_wrapper = NameWrapper::new(name_wrapper_addr, provider);
    let tx = name_wrapper.isWrapped(parent_name_hash);
    let result = provider.call(tx.into_transaction_request()).await?;
    let is_wrapped = isWrappedCall::abi_decode_returns(&result, false)?._0;
    println!("iswrapped: {:?}", result);
    print!("creating subname ... ");
    stdout().flush()?;
    if is_wrapped {
        let tx = name_wrapper.setSubnodeRecord(
            parent_name_hash,
            label.to_owned(),
            sender_addr,
            public_resolver_addr,
            0,
            0,
            0,
        );
        let result =
            provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
        println!("done (txn hash: {:?})", result);
    } else {
        let ens_registry = EnsRegistry::new(ens_registry_addr, provider);
        let tx = ens_registry.setSubnodeRecord(
            parent_name_hash,
            label_hash,
            sender_addr,
            public_resolver_addr,
            0,
        );
        let result =
            provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
        println!("done (txn hash: {:?})", result);
    }
    Ok(())
}

async fn set_resolutions<P: Provider<AnyNetwork>>(
    provider: &P,
    public_resolver_addr: Address,
    complete_name_hash: B256,
    name: String,
    contract_addr: Address,
    is_reverse_claimer: bool,
    sender_addr: Address,
    reverse_registrar_addr: Address,
) -> Result<()> {
    print!("checking if fwd resolution already set ... ");
    stdout().flush()?;
    let public_resolver = PublicResolver::new(public_resolver_addr, provider);
    let tx = public_resolver.addr(complete_name_hash);
    let result = provider.call(tx.into_transaction_request()).await?;
    let result = addrCall::abi_decode_returns(&result, false)?._0;
    println!("result: {:?})", result);

    if result == Address::ZERO {
        print!("setting fwd resolution ({} -> {}) ... ", name, contract_addr);
        stdout().flush()?;
        let tx = public_resolver.setAddr(complete_name_hash, contract_addr);
        let result =
            provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
        println!("done (txn hash: {:?})", result);
    } else {
        println!("fwd resolution already set");
    }

    print!("setting rev resolution ({} -> {}) ... ", contract_addr, name);
    stdout().flush()?;
    if is_reverse_claimer {
        let addr = &(&sender_addr.to_string().to_ascii_lowercase())[2..];
        let reverse_node = namehash(&format!("{}.addr.reverse", addr));
        let tx = public_resolver.setName(reverse_node, name);
        let result =
            provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
        println!("done (txn hash: {:?})", result);
    } else {
        let reverse_registrar = ReverseRegistrar::new(reverse_registrar_addr, provider);
        let tx = reverse_registrar.setNameForAddr(
            contract_addr,
            sender_addr,
            public_resolver_addr,
            name,
        );
        let result =
            provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
        println!("done (txn hash: {:?})", result);
    }

    Ok(())
}

async fn get_config(chain_id: u64) -> Result<ChainConfigResponse> {
    let client = reqwest::Client::new();
    let response = client.get(format!("{}/{}", CONFIG_API_URL, chain_id)).send().await?;

    let status = response.status();
    if !status.is_success() {
        let error: serde_json::Value = response.json().await?;
        eyre::bail!(
            "Contract naming request \
                             failed with status code {status}\n\
                             Details: {error:#}",
        );
    }

    let text = response.text().await?;
    Ok(serde_json::from_str::<ChainConfigResponse>(&text)?)
}

async fn record_metric(metric: Metric) {
    let client = reqwest::Client::new();
    let _ = client.post(METRICS_API_URL).json(&metric).send().await;
}
