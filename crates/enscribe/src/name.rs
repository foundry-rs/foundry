use crate::{
    abi::{
        EnsRegistry, Enscribe, NameWrapper, NameWrapper::isWrappedCall, Ownable, PublicResolver,
        PublicResolver::addrCall, ReverseRegistrar,
    },
    logger::MetricLogger,
};
use alloy_primitives::U256;
use alloy_provider::{network::AnyNetwork, Provider};
use alloy_sol_types::{
    private::{keccak256, Address, B256},
    SolCall,
};
use eyre::Result;
use foundry_common::ens::namehash;
use serde::Deserialize;
use std::{
    io::{stdout, Write},
};

// todo abhi: change this to actual url after api deployed
pub(crate) static CONFIG_API_URL: &str = "http://localhost:3000/api/v1/config";
pub(crate) static AUTO_GEN_NAME_API_URL: &str = "http://localhost:3000/api/v1/name";

const _BASE: u32 = 8453;
const _BASE_SEPOLIA: u32 = 84532;

#[derive(Debug, Deserialize)]
pub struct ChainConfigResponse {
    reverse_registrar_addr: String,
    ens_registry_addr: String,
    public_resolver_addr: String,
    name_wrapper_addr: String,
    enscribe_addr: String,
    parent_name: String,
}

pub async fn set_primary_name<P: Provider<AnyNetwork>>(
    provider: P,
    sender_addr: Address,
    contract_addr: Address,
    name: Option<String>,
    is_reverse_claimer: bool,
    _is_reverse_setter: bool,
    op_type: &str,
) -> Result<()> {
    let chain_id = provider.get_chain_id().await?;
    let config = get_config(chain_id).await?;
    let reverse_registrar_addr: Address = config.reverse_registrar_addr.parse()?;
    let ens_registry_addr: Address = config.ens_registry_addr.parse()?;
    let public_resolver_addr: Address = config.public_resolver_addr.parse()?;
    let name_wrapper_addr: Address = config.name_wrapper_addr.parse()?;
    let is_ownable = is_ownable(&provider, contract_addr).await;

    println!("contract is ownable?: {is_ownable}");

    if let Some(name) = name {
        // let provider = Arc::new(provider);
        let name_splits = name.split('.').collect::<Vec<&str>>();
        let label = name_splits[0];
        let parent = name_splits[1];
        let tld = name_splits[2];

        let logger = MetricLogger::new(
            sender_addr.to_string(),
            chain_id,
            op_type.to_owned(),
            if is_ownable { "Ownable" } else { "ReverseClaimer" }.to_owned(),
            contract_addr.to_string(),
            name.clone()
        );

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
            &logger
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
            &logger
        )
        .await?;
    } else {
        print!("auto generating name ... ");
        stdout().flush()?;
        let label = get_auto_generated_name().await?;
        println!("{label}.{}", config.parent_name);

        let logger = MetricLogger::new(
            sender_addr.to_string(),
            chain_id,
            op_type.to_owned(),
            if is_ownable { "Ownable" } else { "ReverseClaimer" }.to_owned(),
            contract_addr.to_string(),
            format!("{}.{}", label, config.parent_name,)
        );

        enscribe_set_name(
            &provider,
            config.enscribe_addr.parse()?,
            contract_addr,
            &label,
            &config.parent_name,
            &logger
        )
        .await?;
    }

    Ok(())
}

/// checks if the given contract address implements Ownable
async fn is_ownable<P: Provider<AnyNetwork>>(provider: &P, contract_addr: Address) -> bool {
    let ownable = Ownable::new(contract_addr, provider);
    let tx = ownable.owner();
    provider.call(tx.into_transaction_request()).await.is_ok()
}

/// sets name & resolutions via the enscribe contract
async fn enscribe_set_name<P: Provider<AnyNetwork>>(
    provider: &P,
    enscribe_addr: Address,
    contract_addr: Address,
    label: &str,
    parent_name: &str,
    logger: &MetricLogger
) -> Result<()> {
    print!("setting name via enscribe ... ");
    stdout().flush()?;
    let enscribe = Enscribe::new(enscribe_addr, provider);
    let parent_node = namehash(parent_name);
    let tx = enscribe
        .setName(contract_addr, label.to_owned(), parent_name.to_owned(), parent_node)
        .value(U256::from(100000000000000u64));
    let result = provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
    println!("done (txn hash: {:?})", result);
    logger.log("setName", &result.to_string()).await?;
    Ok(())
}

/// creates the subname record
async fn create_subname<P: Provider<AnyNetwork>>(
    sender_addr: Address,
    provider: &P,
    ens_registry_addr: Address,
    public_resolver_addr: Address,
    name_wrapper_addr: Address,
    label: &str,
    parent_name_hash: B256,
    label_hash: B256,
    logger: &MetricLogger
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
        logger.log("createsubname", &result.to_string()).await?;
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
        logger.log("createsubname", &result.to_string()).await?;
    }
    Ok(())
}

/// sets forward & reverse resolutions
async fn set_resolutions<P: Provider<AnyNetwork>>(
    provider: &P,
    public_resolver_addr: Address,
    complete_name_hash: B256,
    name: String,
    contract_addr: Address,
    is_reverse_claimer: bool,
    sender_addr: Address,
    reverse_registrar_addr: Address,
    logger: &MetricLogger
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
        logger.log("fwdres::setAddr", &result.to_string()).await?;
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
        logger.log("revres::setAddr", &result.to_string()).await?;
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
        logger.log("revres::setAddr", &result.to_string()).await?;
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

async fn get_auto_generated_name() -> Result<String> {
    let client = reqwest::Client::new();
    let response = client.get(AUTO_GEN_NAME_API_URL).send().await?;
    Ok(response.text().await?)
}
