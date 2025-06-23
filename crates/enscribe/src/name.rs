use crate::abi::{
    EnsRegistry, EnsRegistry::recordExistsCall, Enscribe, NameWrapper, NameWrapper::isWrappedCall,
    Ownable, Ownable::ownerCall, PublicResolver, PublicResolver::addrCall, ReverseRegistrar,
};
use alloy_chains::NamedChain;
use alloy_ens::namehash;
use alloy_primitives::U256;
use alloy_provider::{
    network::{AnyNetwork, EthereumWallet},
    Provider, ProviderBuilder, WalletProvider,
};
use alloy_sol_types::{
    private::{keccak256, Address, B256},
    SolCall,
};
use eyre::Result;
use foundry_cli::utils;
use foundry_common::sh_println;
use foundry_config::Config;
use names::{Generator, Name};

#[derive(Debug)]
pub struct ChainConfig {
    reverse_registrar_addr: Address,
    ens_registry_addr: Address,
    public_resolver_addr: Address,
    name_wrapper_addr: Address,
    enscribe_addr: Address,
    parent_name: String,
}

pub async fn set_primary_name(
    config: &Config,
    wallet: EthereumWallet,
    contract_addr: Address,
    name: Option<String>,
    _is_reverse_setter: bool,
) -> Result<()> {
    let provider = utils::get_provider(config)?;
    let provider = ProviderBuilder::<_, _, AnyNetwork>::default()
        .with_recommended_fillers()
        .wallet(wallet)
        .connect_provider(provider);

    let sender_addr = provider.default_signer_address();
    let chain_id = provider.get_chain_id().await?;
    let config = get_config(chain_id).await?;
    let reverse_registrar_addr: Address = config.reverse_registrar_addr;
    let ens_registry_addr: Address = config.ens_registry_addr;
    let public_resolver_addr: Address = config.public_resolver_addr;
    let name_wrapper_addr: Address = config.name_wrapper_addr;
    let is_ownable = is_contract_ownable(&provider, contract_addr).await;
    let is_reverse_claimer =
        is_contract_reverse_claimer(&provider, contract_addr, sender_addr, ens_registry_addr)
            .await?;

    // we can't name a contract that isn't Ownable or ReverseClaimer
    if !is_ownable && !is_reverse_claimer {
        sh_println!("Contract doesn't seem to implement Ownable or ReverseClaimer interfaces.")?;
        return Ok(())
    }

    let contract_type = if is_ownable { "Ownable" } else { "ReverseClaimer" }.to_owned();
    sh_println!("Contract is {contract_type} contract.")?;

    if let Some(name) = name {
        let name_splits = name.split('.').collect::<Vec<&str>>();
        let label = name_splits[0];
        let parent = name_splits[1];
        let tld = name_splits[2];

        let parent_name = format!("{parent}.{tld}");
        let parent_name_hash = namehash(&parent_name);
        let label_hash = keccak256(label);
        let complete_name_hash = namehash(&name);

        if !name_already_registered(&provider, complete_name_hash, ens_registry_addr).await? {
            create_subname(
                &provider,
                sender_addr,
                ens_registry_addr,
                public_resolver_addr,
                name_wrapper_addr,
                label,
                parent_name_hash,
                label_hash,
            )
            .await?;
        }

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
    } else {
        sh_println!("auto generating name ...")?;
        let label = get_auto_generated_name();
        sh_println!("{label}.{}", config.parent_name)?;

        enscribe_set_name(
            &provider,
            config.enscribe_addr,
            contract_addr,
            &label,
            &config.parent_name,
        )
        .await?;
    }

    sh_println!()?;
    sh_println!("✨ Contract named: https://app.enscribe.xyz/explore/{chain_id}/{contract_addr}")?;

    Ok(())
}

/// checks if the given contract address implements Ownable
async fn is_contract_ownable<P: Provider<AnyNetwork>>(
    provider: &P,
    contract_addr: Address,
) -> bool {
    let ownable = Ownable::new(contract_addr, provider);
    let tx = ownable.owner();
    provider.call(tx.into_transaction_request()).await.is_ok()
}

/// checks if the given contract address implements Ownable
async fn is_contract_reverse_claimer<P: Provider<AnyNetwork>>(
    provider: &P,
    contract_addr: Address,
    sender_addr: Address,
    ens_registry_addr: Address,
) -> Result<bool> {
    let addr = &(&contract_addr.to_string().to_ascii_lowercase())[2..];
    let reverse_node = namehash(&format!("{addr}.addr.reverse"));
    let ens_registry = EnsRegistry::new(ens_registry_addr, provider);
    let tx = ens_registry.owner(reverse_node);
    let result = provider.call(tx.into_transaction_request()).await?;
    let addr = ownerCall::abi_decode_returns(&result)?;
    Ok(addr == sender_addr)
}

/// sets name & resolutions via the enscribe contract
async fn enscribe_set_name<P: Provider<AnyNetwork>>(
    provider: &P,
    enscribe_addr: Address,
    contract_addr: Address,
    label: &str,
    parent_name: &str,
) -> Result<()> {
    sh_println!("setting name via enscribe ...")?;
    let enscribe = Enscribe::new(enscribe_addr, provider);
    let parent_node = namehash(parent_name);
    let tx = enscribe
        .setName(contract_addr, label.to_owned(), parent_name.to_owned(), parent_node)
        .value(U256::from(100000000000000u64));
    let result = provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
    sh_println!("done (txn hash: {:?})", result)?;
    Ok(())
}

/// probes the ens registry to check if the given `name` is already registered on the chain
async fn name_already_registered<P: Provider<AnyNetwork>>(
    provider: &P,
    name: B256,
    ens_registry_addr: Address,
) -> Result<bool> {
    let ens_registry = EnsRegistry::new(ens_registry_addr, provider);
    let tx = ens_registry.recordExists(name);
    let result = provider.call(tx.into_transaction_request()).await?;
    let is_name_exists = recordExistsCall::abi_decode_returns(&result)?;
    Ok(is_name_exists)
}

/// creates the subname record
#[expect(clippy::too_many_arguments)]
async fn create_subname<P: Provider<AnyNetwork>>(
    provider: &P,
    sender_addr: Address,
    ens_registry_addr: Address,
    public_resolver_addr: Address,
    name_wrapper_addr: Address,
    label: &str,
    parent_name_hash: B256,
    label_hash: B256,
) -> Result<()> {
    // for Base chains, handle subname creation differently
    let chain_id = provider.get_chain_id().await?;
    if chain_id == NamedChain::Base as u64 || chain_id == NamedChain::BaseSepolia as u64 {
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
        sh_println!("done (txn hash: {:?})", result)?;
        return Ok(());
    }

    // check if parent domain (e.g. abhi.eth) is wrapped or unwrapped
    let name_wrapper = NameWrapper::new(name_wrapper_addr, provider);
    let tx = name_wrapper.isWrapped(parent_name_hash);
    let result = provider.call(tx.into_transaction_request()).await?;
    let is_wrapped = isWrappedCall::abi_decode_returns(&result)?;
    sh_println!("creating subname ...")?;
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
        sh_println!("done (txn hash: {:?})", result)?;
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
        sh_println!("done (txn hash: {:?})", result)?;
    }
    Ok(())
}

/// sets forward & reverse resolutions
#[expect(clippy::too_many_arguments)]
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
    sh_println!("checking if fwd resolution already set ...")?;
    let public_resolver = PublicResolver::new(public_resolver_addr, provider);
    let tx = public_resolver.addr(complete_name_hash);
    let result = provider.call(tx.into_transaction_request()).await?;
    let result = addrCall::abi_decode_returns(&result)?;

    if result == Address::ZERO {
        sh_println!("setting fwd resolution ({} -> {}) ...", name, contract_addr)?;
        let tx = public_resolver.setAddr(complete_name_hash, contract_addr);
        let result =
            provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
        sh_println!("done (txn hash: {:?})", result)?;
    } else {
        sh_println!("fwd resolution already set")?;
    }

    sh_println!("setting rev resolution ({} -> {}) ...", contract_addr, name)?;
    if is_reverse_claimer {
        let addr = &(&sender_addr.to_string().to_ascii_lowercase())[2..];
        let reverse_node = namehash(&format!("{addr}.addr.reverse"));
        let tx = public_resolver.setName(reverse_node, name);
        let result =
            provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
        sh_println!("done (txn hash: {:?})", result)?;
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
        sh_println!("done (txn hash: {:?})", result)?;
    }

    Ok(())
}

/// fetches the chain config for `chaind_id` from the Enscribe API
async fn get_config(chain_id: u64) -> Result<ChainConfig> {
    let chain = NamedChain::try_from(chain_id)?;

    let reverse_registrar_addr = chain
        .reverse_registrar_address()
        .ok_or_else(|| eyre::eyre!("reverse registrar address not found"))?;
    let ens_registry_addr = chain
        .ens_registry_address()
        .ok_or_else(|| eyre::eyre!("ens registry address not found"))?;
    let public_resolver_addr = chain
        .public_resolver_address()
        .ok_or_else(|| eyre::eyre!("ens registry address not found"))?;
    let name_wrapper_addr = chain
        .name_wrapper_address()
        .ok_or_else(|| eyre::eyre!("name wrapper address not found"))?;
    let enscribe_addr =
        chain.enscribe_address().ok_or_else(|| eyre::eyre!("enscribe address not found"))?;
    let parent_name = chain.parent_name().ok_or_else(|| eyre::eyre!("parent name not found"))?;

    Ok(ChainConfig {
        reverse_registrar_addr,
        ens_registry_addr,
        public_resolver_addr,
        name_wrapper_addr,
        enscribe_addr,
        parent_name,
    })
}

/// fetches a random name from the Enscribe API
fn get_auto_generated_name() -> String {
    let mut generator = Generator::with_naming(Name::Numbered);
    generator.next().unwrap()
}
