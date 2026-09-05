use super::{auth::confirm_and_build, print_result_line};
use crate::tx::{CastTxBuilder, read_only_sender};
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, Network};
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{RpcOpts, TransactionOpts},
    utils::{LoadConfig, parse_ether_value},
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder};
use foundry_wallets::{BrowserWalletOpts, WalletOpts};
use std::str::FromStr;
use tempo_alloy::TempoNetwork;

/// CLI arguments for `cast estimate`.
#[derive(Debug, Parser)]
pub struct EstimateArgs {
    /// The destination of the transaction.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(allow_negative_numbers = true)]
    args: Vec<String>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long, short = 'B')]
    block: Option<BlockId>,

    /// Calculate the cost of a transaction using the network gas price.
    ///
    /// If not specified the amount of gas will be estimated.
    #[arg(long)]
    cost: bool,

    #[command(flatten)]
    wallet: WalletOpts,

    #[command(flatten)]
    browser: BrowserWalletOpts,

    #[command(subcommand)]
    command: Option<EstimateSubcommands>,

    #[command(flatten)]
    tx: TransactionOpts,

    /// Skip the EIP-7702 authorization disclosure confirmation.
    #[arg(long)]
    force: bool,

    #[command(flatten)]
    rpc: RpcOpts,
}

#[derive(Debug, Parser)]
pub enum EstimateSubcommands {
    /// Estimate gas cost to deploy a smart contract
    #[command(name = "--create")]
    Create {
        /// The bytecode of contract
        code: String,

        /// The signature of the constructor
        sig: Option<String>,

        /// Constructor arguments
        #[arg(allow_negative_numbers = true)]
        args: Vec<String>,

        /// Ether to send in the transaction
        ///
        /// Either specified in wei, or as a string with a unit type:
        ///
        /// Examples: 1ether, 10gwei, 0.01ether
        #[arg(long, value_parser = parse_ether_value)]
        value: Option<U256>,
    },
}

impl EstimateArgs {
    pub async fn run(self) -> Result<()> {
        if self.tx.tempo.is_tempo() {
            self.run_with_network::<TempoNetwork>().await
        } else {
            self.run_with_network::<Ethereum>().await
        }
    }

    async fn run_with_network<N: Network>(self) -> Result<()>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let Self {
            to,
            mut sig,
            mut args,
            mut tx,
            block,
            cost,
            wallet,
            browser,
            force,
            rpc,
            command,
        } = self;

        let config = rpc.load_config()?;
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
        let (sender, is_browser) = read_only_sender::<N>(&browser, wallet).await?;

        let code = if let Some(EstimateSubcommands::Create {
            code,
            sig: create_sig,
            args: create_args,
            value,
        }) = command
        {
            sig = create_sig;
            args = create_args;
            if let Some(value) = value {
                tx.value = Some(value);
            }
            Some(code)
        } else {
            None
        };

        let builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .raw();
        let Some(tx) = confirm_and_build(builder, sender, force, None, true).await? else {
            return Ok(());
        };

        let tx = if is_browser { tx.browser_wallet_gas_estimation_request() } else { tx };
        let gas = provider.estimate_gas(tx).block(block.unwrap_or_default()).await?;
        if cost {
            let cost = provider.get_gas_price().await? * gas as u128;
            print_result_line(cost as f64 / 1e18)
        } else {
            print_result_line(gas)
        }
    }
}
