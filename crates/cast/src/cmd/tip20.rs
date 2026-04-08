use crate::{
    cmd::{erc20::build_provider_with_signer, send::cast_send},
    tx::{CastTxSender, SendTxOpts},
};
use alloy_ens::NameOrAddress;
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{B256, U256};
use alloy_provider::Provider;
use alloy_sol_types::sol;
use clap::{Args, Parser};
use foundry_cli::{
    opts::{RpcOpts, TempoOpts},
    utils::{LoadConfig, get_chain},
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder};
use std::str::FromStr;
use tempo_alloy::TempoNetwork;
use tempo_contracts::precompiles::{TIP20_FACTORY_ADDRESS, is_iso4217_currency};

sol! {
    #[sol(rpc)]
    interface ITIP20Factory {
        function createToken(
            string memory name,
            string memory symbol,
            string memory currency,
            address quoteToken,
            address admin,
            bytes32 salt
        ) external returns (address token);
    }
}

/// Returns a warning message for non-ISO 4217 currency codes used in TIP-20 token creation.
fn iso4217_warning_message(currency: &str) -> String {
    let hyperlink = |url: &str| format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\");
    let tip20_docs = hyperlink("https://docs.tempo.xyz/protocol/tip20/overview");
    let iso_docs = hyperlink("https://www.iso.org/iso-4217-currency-codes.html");

    format!(
        "\"{currency}\" is not a recognized ISO 4217 currency code.\n\
         \n\
         If the token you are trying to deploy is a fiat-backed stablecoin, Tempo strongly\n\
         recommends that the currency code field be the ISO-4217 currency code of the fiat\n\
         currency your token tracks (e.g. \"USD\", \"EUR\", \"GBP\").\n\
         \n\
         The currency field is IMMUTABLE after token creation and affects fee payment\n\
         eligibility, DEX routing, and quote token pairing. Only \"USD\"-denominated tokens\n\
         can be used to pay transaction fees on Tempo.\n\
         \n\
         Learn more:\n  \
         - Tempo TIP-20 docs: {tip20_docs}\n  \
         - ISO 4217 standard: {iso_docs}"
    )
}

/// TIP-20 token operations (Tempo).
#[derive(Debug, Parser, Clone)]
pub enum Tip20Subcommand {
    /// Create a new TIP-20 token via the TIP20Factory.
    #[command(visible_alias = "c")]
    Create {
        /// The token name (e.g. "US Dollar Coin").
        name: String,

        /// The token symbol (e.g. "USDC").
        symbol: String,

        /// The ISO 4217 currency code (e.g. "USD", "EUR", "GBP").
        /// This field is IMMUTABLE after creation and affects fee payment
        /// eligibility, DEX routing, and quote token pairing.
        currency: String,

        /// The TIP-20 quote token address used for exchange pricing.
        #[arg(value_parser = NameOrAddress::from_str)]
        quote_token: NameOrAddress,

        /// The admin address to receive DEFAULT_ADMIN_ROLE on the new token.
        #[arg(value_parser = NameOrAddress::from_str)]
        admin: NameOrAddress,

        /// A unique salt for deterministic address derivation (hex-encoded bytes32).
        salt: B256,

        /// Skip the ISO 4217 currency code validation warning.
        #[arg(long)]
        force: bool,

        #[command(flatten)]
        send_tx: SendTxOpts,

        #[command(flatten)]
        tx: Tip20TxOpts,
    },
}

/// Transaction options for TIP-20 operations.
#[derive(Debug, Clone, Args)]
#[command(next_help_heading = "Transaction options")]
pub struct Tip20TxOpts {
    /// Gas limit for the transaction.
    #[arg(long)]
    pub gas_limit: Option<U256>,

    /// Gas price or max fee per gas for the transaction.
    #[arg(long)]
    pub gas_price: Option<U256>,

    /// Max priority fee per gas (EIP-1559).
    #[arg(long)]
    pub priority_gas_price: Option<U256>,

    /// Nonce for the transaction.
    #[arg(long)]
    pub nonce: Option<U256>,

    #[command(flatten)]
    pub tempo: TempoOpts,
}

impl Tip20TxOpts {
    /// Applies gas, fee, nonce, and Tempo options to a transaction request.
    fn apply<N: Network>(&self, tx: &mut N::TransactionRequest, legacy: bool)
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        if let Some(gas_limit) = self.gas_limit {
            tx.set_gas_limit(gas_limit.to());
        }

        if let Some(gas_price) = self.gas_price {
            if legacy {
                tx.set_gas_price(gas_price.to());
            } else {
                tx.set_max_fee_per_gas(gas_price.to());
            }
        }

        if !legacy && let Some(priority_fee) = self.priority_gas_price {
            tx.set_max_priority_fee_per_gas(priority_fee.to());
        }

        self.tempo.apply::<N>(tx, self.nonce.map(|n| n.to()));
    }
}

impl Tip20Subcommand {
    fn rpc_opts(&self) -> &RpcOpts {
        match self {
            Self::Create { send_tx, .. } => &send_tx.eth.rpc,
        }
    }

    pub async fn run(self) -> eyre::Result<()> {
        let (signer, tempo_access_key) = match &self {
            Self::Create { send_tx, .. } => {
                if send_tx.eth.wallet.from.is_some() {
                    send_tx.eth.wallet.maybe_signer().await?
                } else {
                    (None, None)
                }
            }
        };

        let config = self.rpc_opts().load_config()?;

        match self {
            Self::Create {
                name,
                symbol,
                currency,
                quote_token,
                admin,
                salt,
                force,
                send_tx,
                tx: tx_opts,
            } => {
                if !is_iso4217_currency(&currency) && !force {
                    sh_warn!("{}", iso4217_warning_message(&currency))?;
                    let response: String = foundry_common::prompt!("\nContinue anyway? [y/N] ")?;
                    if !matches!(response.trim(), "y" | "Y") {
                        sh_println!("Aborted.")?;
                        return Ok(());
                    }
                }

                let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);
                let provider = ProviderBuilder::<TempoNetwork>::from_config(&config)?.build()?;
                let quote_token_addr = quote_token.resolve(&provider).await?;
                let admin_addr = admin.resolve(&provider).await?;

                let mut tx = ITIP20Factory::new(TIP20_FACTORY_ADDRESS, &provider)
                    .createToken(name, symbol, currency, quote_token_addr, admin_addr, salt)
                    .into_transaction_request();

                tx_opts.apply::<TempoNetwork>(
                    &mut tx,
                    get_chain(config.chain, &provider).await?.is_legacy(),
                );

                if let Some(ref access_key) = tempo_access_key {
                    let signer = signer.as_ref().expect("signer required for access key");
                    tx.set_from(access_key.wallet_address);
                    tx.set_key_id(access_key.key_address);

                    let raw_tx = tx
                        .sign_with_access_key_provisioning(
                            &provider,
                            signer,
                            access_key.wallet_address,
                            access_key.key_address,
                            access_key.key_authorization.as_ref(),
                        )
                        .await?;

                    let tx_hash = *provider.send_raw_transaction(&raw_tx).await?.tx_hash();
                    let cast = CastTxSender::new(&provider);
                    cast.print_tx_result(
                        tx_hash,
                        send_tx.cast_async,
                        send_tx.confirmations,
                        timeout,
                    )
                    .await?
                } else {
                    let signer = signer.unwrap_or(send_tx.eth.wallet.signer().await?);
                    let provider = build_provider_with_signer::<TempoNetwork>(&send_tx, signer)?;
                    cast_send(
                        provider,
                        tx,
                        send_tx.cast_async,
                        send_tx.sync,
                        send_tx.confirmations,
                        timeout,
                    )
                    .await?
                }
            }
        };
        Ok(())
    }
}
