use super::{
    auth::{confirm_and_build, confirm_and_build_with_tempo_wallet},
    print_result_line,
};
use crate::{
    tempo,
    tx::{CastTxBuilder, SenderKind, read_only_sender},
};
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
use foundry_common::{
    FoundryTransactionBuilder,
    provider::ProviderBuilder,
    tempo::{configurable_account::prepare_request, mark_sponsored_gas_estimation},
};
use foundry_config::Config;
use foundry_wallets::{BrowserWalletOpts, WalletOpts};
use std::{str::FromStr, sync::Arc};
use tempo_alloy::TempoNetwork;

#[cfg(feature = "base")]
use base_common_network::Base;

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
        let config = self.rpc.load_config()?;
        let requires_tempo = self.tx.tempo.is_tempo()
            || self.tx.tempo.session_id()?.is_some()
            || self.wallet.tempo_access_key.is_some();
        let network = super::resolve_transaction_network(&config, requires_tempo).await?;
        if network.is_tempo() {
            return self.run_with_network::<TempoNetwork>(config).await;
        }
        #[cfg(feature = "base")]
        if network.is_base() {
            super::validate_base_transaction_options(&self.tx)?;
            return self.run_with_network::<Base>(config).await;
        }
        self.run_with_network::<Ethereum>(config).await
    }

    async fn run_with_network<N: Network>(self, config: Config) -> Result<()>
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
            rpc: _,
            command,
        } = self;

        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
        let chain_id = match config.chain {
            Some(chain) => chain.id(),
            None => provider.get_chain_id().await?,
        };
        // `cast send` rejects the same conflict.
        if let (Some(account), Some(from)) = (tx.tempo.configurable_account, wallet.from) {
            eyre::ensure!(
                account == from,
                "--tempo.configurable-account {account} does not match the sender {from}"
            );
        }
        let as_access_key = tx.tempo.configurable_access_key.is_some();
        let simulation_wallet = wallet.clone();
        let (sender, is_browser, access_key) =
            read_only_sender::<N>(&browser, wallet, &tx, chain_id).await?;
        // Like `cast send`, an access key signs for its root account and is estimated with its key.
        if let Some(access_key) = &access_key {
            eyre::ensure!(
                !tx.tempo.has_configurable_account_options(),
                "Tempo configurable accounts cannot be combined with an access key"
            );
            tx.tempo.key_id = Some(access_key.key_id()?);
        }
        let sponsored = tx.tempo.is_sponsored();
        let sponsor = tx.tempo.sponsor.filter(|_| !config.eth_rpc_curl);
        // Share the resolved signer with the configurable account lookup.
        let (sender, signer) = match sender {
            SenderKind::OwnedSigner(signer) => (None, Some(Arc::new(*signer))),
            sender => (Some(sender), None),
        };
        let mut sender = match (tx.tempo.configurable_account, sender) {
            (Some(account), _) if !is_browser => account.into(),
            (_, Some(sender)) => sender,
            (_, None) => SenderKind::Signer(signer.as_deref().expect("an owned signer")),
        };
        // A configurable account sender is estimated with its owners' approvals.
        let simulation = if is_browser || access_key.is_some() || sender.address().is_zero() {
            None
        } else {
            tempo::configurable_account_simulation::<N>(
                &tx.tempo,
                &config,
                &simulation_wallet,
                sender.address(),
                signer.clone(),
                block.unwrap_or_default(),
            )
            .await?
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

        let mut builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .raw();
        if let Some((account, simulation)) = simulation {
            // A configurable access key signs for the sender as its `keyId`.
            let from = if as_access_key {
                builder.tx_mut().set_key_id(account);
                sender.address()
            } else {
                sender = account.into();
                account
            };
            prepare_request::<N>(builder.tx_mut(), from, simulation);
        }
        let tx = if let Some(access_key) = &access_key {
            confirm_and_build_with_tempo_wallet(builder, access_key, force, None)
                .await?
                .map(|(tx, _)| tx)
        } else {
            confirm_and_build(builder, sender, force, None, true).await?
        };
        let Some(tx) = tx else { return Ok(()) };

        let mut tx = if is_browser { tx.browser_wallet_gas_estimation_request() } else { tx };
        // Estimated as `cast send` estimates a sponsored transaction.
        if sponsored {
            mark_sponsored_gas_estimation(&provider, &mut tx, sponsor).await;
        }
        let gas = provider.estimate_gas(tx).block(block.unwrap_or_default()).await?;
        if cost {
            let cost = provider.get_gas_price().await? * gas as u128;
            print_result_line(cost as f64 / 1e18)
        } else {
            print_result_line(gas)
        }
    }
}
