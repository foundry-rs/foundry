use alloy_network::{AnyNetwork, EthereumWallet};
use alloy_primitives::Address;
use alloy_provider::{ProviderBuilder, WalletProvider};
use clap::Parser;
use foundry_cli::{opts::EthereumOpts, utils, utils::LoadConfig};
use foundry_config::{
    figment,
    figment::{
        value::{Dict, Map},
        Metadata, Profile,
    },
    merge_impl_figment_convert, Config,
};

merge_impl_figment_convert!(NameArgs, eth);

/// CLI arguments for `forge name`.
#[derive(Clone, Debug, Parser)]
pub struct NameArgs {
    /// The name to set.
    #[arg(long)]
    pub ens_name: String,

    #[arg(long)]
    pub auto_name: bool,

    /// The address of the contract.
    #[arg(long)]
    pub contract_address: Address,

    /// Whether the contract is ReverseClaimable or not.
    #[arg(long, requires = "ens_name")]
    pub reverse_claimer: bool,

    #[command(flatten)]
    eth: EthereumOpts,
}

impl NameArgs {
    pub async fn run(self) -> eyre::Result<()> {
        println!("args: {:?}", self);
        let config = self.load_config()?;
        let signer = self.eth.wallet.signer().await?;
        let provider = utils::get_provider(&config)?;
        let provider = ProviderBuilder::<_, _, AnyNetwork>::default()
            .with_recommended_fillers()
            .wallet(EthereumWallet::new(signer))
            .on_provider(provider);
        let sender_addr = provider.default_signer_address();
        
        enscribe::set_primary_name(
            provider,
            sender_addr,
            self.contract_address,
            self.ens_name,
            self.reverse_claimer,
        )
        .await?;
        Ok(())
    }
}

impl figment::Provider for NameArgs {
    fn metadata(&self) -> Metadata {
        Metadata::named("Name Args Provider")
    }

    fn data(&self) -> eyre::Result<Map<Profile, Dict>, figment::Error> {
        let dict = Dict::default();
        Ok(Map::from([(Config::selected_profile(), dict)]))
    }
}
