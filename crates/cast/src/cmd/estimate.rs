use super::auth::confirm_auth_rpc_disclosure;
#[cfg(feature = "base")]
use crate::cmd::resolve_network;
use crate::tx::{CastTxBuilder, SenderKind};
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, Network};
use alloy_primitives::U256;
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
#[cfg(feature = "base")]
use base_common_network::Base as BaseNetwork;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::print_scalar,
    opts::{RpcOpts, TransactionOpts},
    utils::{LoadConfig, parse_ether_value},
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder, shell};
use foundry_wallets::{BrowserWalletOpts, WalletOpts};
use serde::Serialize;
use std::{fmt::Display, str::FromStr};
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
            return self.run_with_network::<TempoNetwork>().await;
        }

        #[cfg(feature = "base")]
        if resolve_network(&self.rpc.load_config()?).await?.is_base() {
            return self.run_with_network::<BaseNetwork>().await;
        }

        self.run_with_network::<Ethereum>().await
    }

    pub async fn run_with_network<N: Network>(self) -> Result<()>
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
        let browser = browser.run::<N>().await?;
        let sender = if let Some(browser) = &browser {
            browser.address().into()
        } else {
            SenderKind::from_wallet_opts(wallet).await?
        };

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
        if builder.has_auth() && !confirm_auth_rpc_disclosure(&builder, &sender, force)? {
            return Ok(());
        }
        let (tx, _) = builder.build(sender).await?;

        let tx = if browser.is_some() { tx.browser_wallet_gas_estimation_request() } else { tx };
        let gas = provider.estimate_gas(tx).block(block.unwrap_or_default()).await?;
        if cost {
            let gas_price_wei = provider.get_gas_price().await?;
            let cost = gas_price_wei * gas as u128;
            let cost_eth = cost as f64 / 1e18;
            print_estimate_result(cost_eth)?;
        } else {
            print_estimate_result(gas)?;
        }
        Ok(())
    }
}

fn print_estimate_result(value: impl Serialize + Display) -> Result<()> {
    if shell::is_json() {
        print_scalar(value)
    } else {
        // Bypass the shell verbosity layer so `--quiet` does not suppress the primary result.
        let mut shell = shell::Shell::get();
        let out = shell.out();
        writeln!(out, "{value}")?;
        out.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_estimate_value() {
        let args: EstimateArgs = EstimateArgs::parse_from(["foundry-cli", "--value", "100"]);
        assert!(args.tx.value.is_some());
    }
}
