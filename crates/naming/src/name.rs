use std::io::{stdout, Write};
use std::sync::Arc;
use alloy_provider::network::AnyNetwork;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::private::{keccak256, Address, B256};
use clap::Parser;
use foundry_common::ens::namehash;
use crate::abi::{EnsRegistry, NameWrapper, PublicResolver, ReverseRegistrar};
use eyre::Result;
use crate::abi::NameWrapper::isWrappedCall;
use alloy_sol_types::SolCall;
use foundry_cli::opts::EthereumOpts;

#[derive(Clone, Debug, Parser)]
pub struct NamingArgs {
    /// The name to set.
    pub ens_name: String,

    /// The address of the contract.
    pub address: Address,

    pub sender_addr: Address,

    /// Whether the contract is ReverseClaimable or not.
    #[arg(long)]
    pub reverse_claimer: bool,

    #[command(flatten)]
    pub eth: EthereumOpts
}

impl NamingArgs {
    pub async fn run(self) -> Result<()> {
        Self::set_primary_name(self.eth.wallet.raw.private_key.unwrap(),
                               self.sender_addr,
                               self.address,
                               self.ens_name,
                               self.reverse_claimer
        ).await
    }

    pub async fn set_primary_name(
        key: String,
        sender_addr: Address,
        contract_addr: Address,
        name: String,
        reverseclaimable: bool,
    ) -> Result<()> {
        let signer = key.parse::<PrivateKeySigner>()?;
        // todo abhi: pass in a provider
        let provider = Arc::new(
            ProviderBuilder::<_, _, AnyNetwork>::default()
                .wallet(signer)
                .connect("https://sepolia.drpc.org")
                .await?,
        );
        // .on_provider(provider);
        // .connect_http(rpc_url.parse()?);

        let reverse_registrar_addr: Address =
            "0xCF75B92126B02C9811d8c632144288a3eb84afC8".parse()?;
        let ens_registry_addr: Address = "0x00000000000C2E074eC69A0dFb2997BA6C7d2e1e".parse()?;
        let public_resolver_addr: Address = "0x8948458626811dd0c23EB25Cc74291247077cC51".parse()?;
        let name_wrapper_addr: Address = "0x0635513f179D50A207757E05759CbD106d7dFcE8".parse()?;

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

        Self::create_subname(
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

        Self::set_resolutions(
            &provider,
            public_resolver_addr,
            complete_name_hash,
            name,
            contract_addr,
            reverseclaimable,
            sender_addr,
            reverse_registrar_addr,
        )
            .await?;

        Ok(())
    }

    async fn create_subname<P: Provider<AnyNetwork>>(
        sender_addr: Address,
        provider: &Arc<P>,
        ens_registry_addr: Address,
        public_resolver_addr: Address,
        name_wrapper_addr: Address,
        label: &str,
        parent_name_hash: B256,
        label_hash: B256,
    ) -> Result<()> {
        // check if parent domain (e.g. abhi.eth) is wrapped or unwrapped
        let name_wrapper = NameWrapper::new(name_wrapper_addr, provider.clone());
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
            let ens_registry = EnsRegistry::new(ens_registry_addr, provider.clone());
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
        provider: &Arc<P>,
        public_resolver_addr: Address,
        complete_name_hash: B256,
        name: String,
        contract_addr: Address,
        reverse_claimable: bool,
        sender_addr: Address,
        reverse_registrar_addr: Address,
    ) -> Result<()> {
        print!("checking if fwd resolution already set ... ");
        stdout().flush()?;
        let public_resolver = PublicResolver::new(public_resolver_addr, provider.clone());
        let tx = public_resolver.addr(complete_name_hash);
        let result = provider.call(tx.into_transaction_request()).await?;
        println!("result: {:?})", result);

        print!("setting fwd resolution ({} -> {}) ... ", name, contract_addr);
        stdout().flush()?;
        let tx = public_resolver.setAddr(complete_name_hash, contract_addr);
        let result =
            provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
        println!("done (txn hash: {:?})", result);

        print!("setting rev resolution ({} -> {}) ... ", contract_addr, name);
        stdout().flush()?;
        let reverse_claimable = reverse_claimable;
        if reverse_claimable {
            let addr = &(&sender_addr.to_string().to_ascii_lowercase())[2..];
            let reverse_node = namehash(&format!("{}.addr.reverse", addr));
            let tx = public_resolver.setName(reverse_node, name);
            let result =
                provider.send_transaction(tx.into_transaction_request()).await?.watch().await?;
            println!("done (txn hash: {:?})", result);
        } else {
            let reverse_registrar = ReverseRegistrar::new(reverse_registrar_addr, provider.clone());
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
}