use alloy_network::EthereumWallet;
use alloy_primitives::Address;
use clap::Parser;
use foundry_cli::{opts::EthereumOpts, utils::LoadConfig};
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
    /// The ENS name to set.
    #[arg(long)]
    pub ens_name: Option<String>,

    /// The address of the contract.
    #[arg(long)]
    pub contract_address: Address,

    #[command(flatten)]
    eth: EthereumOpts,
}

impl NameArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let config = self.load_config()?;
        let signer = self.eth.wallet.signer().await?;

        enscribe::set_primary_name(
            &config,
            EthereumWallet::new(signer),
            self.contract_address,
            self.ens_name,
            false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_parse_contract_and_name_args() {
        let args: NameArgs = NameArgs::parse_from([
            "foundry-cli",
            "--contract-address",
            "0x3fAB184622Dc19b6109349B94811493BF2a45362",
            "--ens-name",
            "test.abhi.eth",
        ]);

        assert_eq!(
            args.contract_address,
            "0x3fAB184622Dc19b6109349B94811493BF2a45362".parse::<Address>().unwrap()
        );
        assert_eq!(args.ens_name, Some("test.abhi.eth".to_owned()));
    }
}
