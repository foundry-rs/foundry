use std::{path::PathBuf, str::FromStr, time::Duration};

use alloy_consensus::{SignableTransaction, Signed};
use alloy_ens::NameOrAddress;
use alloy_network::{AnyNetwork, EthereumWallet, Network};
use alloy_provider::{Provider, ProviderBuilder as AlloyProviderBuilder};
use alloy_signer::{Signature, Signer};
use clap::Parser;
use eyre::{Result, eyre};
use foundry_cli::{opts::TransactionOpts, utils::LoadConfig};
use foundry_common::{
    fmt::{UIfmt, UIfmtReceiptExt},
    provider::ProviderBuilder,
};
use foundry_primitives::FoundryTransactionBuilder;
use tempo_alloy::TempoNetwork;

use crate::tx::{self, CastTxBuilder, CastTxSender, SendTxOpts};

/// CLI arguments for `cast send`.
#[derive(Debug, Parser)]
pub struct SendTxArgs {
    /// The destination of the transaction.
    ///
    /// If not provided, you must use cast send --create.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(allow_negative_numbers = true)]
    args: Vec<String>,

    /// Raw hex-encoded data for the transaction. Used instead of \[SIG\] and \[ARGS\].
    #[arg(
        long,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

    #[command(flatten)]
    send_tx: SendTxOpts,

    #[command(subcommand)]
    command: Option<SendTxSubcommands>,

    /// Send via `eth_sendTransaction` using the `--from` argument or $ETH_FROM as sender
    #[arg(long, requires = "from")]
    unlocked: bool,

    #[command(flatten)]
    tx: TransactionOpts,

    /// The path of blob data to be sent.
    #[arg(
        long,
        value_name = "BLOB_DATA_PATH",
        conflicts_with = "legacy",
        requires = "blob",
        help_heading = "Transaction options"
    )]
    path: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub enum SendTxSubcommands {
    /// Use to deploy raw contract bytecode.
    #[command(name = "--create")]
    Create {
        /// The bytecode of the contract to deploy.
        code: String,

        /// The signature of the function to call.
        sig: Option<String>,

        /// The arguments of the function to call.
        #[arg(allow_negative_numbers = true)]
        args: Vec<String>,
    },
}

impl SendTxArgs {
    pub async fn run(self) -> Result<()> {
        if self.tx.tempo.is_tempo() {
            self.run_generic::<TempoNetwork>().await
        } else {
            self.run_generic::<AnyNetwork>().await
        }
    }

    pub async fn run_generic<N: Network>(self) -> Result<()>
    where
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
    {
        let Self { to, mut sig, mut args, data, send_tx, tx, command, unlocked, path } = self;

        let print_sponsor_hash = tx.tempo.print_sponsor_hash;

        let blob_data = if let Some(path) = path { Some(std::fs::read(path)?) } else { None };

        if let Some(data) = data {
            sig = Some(data);
        }

        let code = if let Some(SendTxSubcommands::Create {
            code,
            sig: constructor_sig,
            args: constructor_args,
        }) = command
        {
            // ensure we don't violate settings for transactions that can't be CREATE: 7702 and 4844
            // which require mandatory target
            if to.is_none() && !tx.auth.is_empty() {
                return Err(eyre!(
                    "EIP-7702 transactions can't be CREATE transactions and require a destination address"
                ));
            }
            // ensure we don't violate settings for transactions that can't be CREATE: 7702 and 4844
            // which require mandatory target
            if to.is_none() && blob_data.is_some() {
                return Err(eyre!(
                    "EIP-4844 transactions can't be CREATE transactions and require a destination address"
                ));
            }

            sig = constructor_sig;
            args = constructor_args;
            Some(code)
        } else {
            None
        };

        let config = send_tx.eth.load_config()?;
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;

        if let Some(interval) = send_tx.poll_interval {
            provider.client().set_poll_interval(Duration::from_secs(interval))
        }

        let builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;

        // If --tempo.print-sponsor-hash was passed, build the tx, print the hash, and exit.
        if print_sponsor_hash {
            let from = send_tx.eth.wallet.from.unwrap_or(config.sender);
            let (tx, _) = builder.build(from).await?;
            let hash = tx
                .compute_sponsor_hash(from)
                .ok_or_else(|| eyre!("This network does not support sponsored transactions"))?;
            sh_println!("{hash:?}")?;
            return Ok(());
        }

        let timeout = send_tx.timeout.unwrap_or(config.transaction_timeout);

        // Launch browser signer if `--browser` flag is set
        let browser = send_tx.browser.run::<N>().await?;

        // Case 1:
        // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
        // This should be the only way this RPC method is used as it requires a local node
        // or remote RPC with unlocked accounts.
        if unlocked && browser.is_none() {
            // only check current chain id if it was specified in the config
            if let Some(config_chain) = config.chain {
                let current_chain_id = provider.get_chain_id().await?;
                let config_chain_id = config_chain.id();
                // switch chain if current chain id is not the same as the one specified in the
                // config
                if config_chain_id != current_chain_id {
                    sh_warn!("Switching to chain {}", config_chain)?;
                    provider
                        .raw_request::<_, ()>(
                            "wallet_switchEthereumChain".into(),
                            [serde_json::json!({
                                "chainId": format!("0x{:x}", config_chain_id),
                            })],
                        )
                        .await?;
                }
            }

            let (tx, _) = builder.build(config.sender).await?;

            cast_send(
                provider,
                tx,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
            )
            .await
        // Case 2:
        // Browser wallet signs and sends the transaction in one step.
        } else if let Some(browser) = browser {
            let (tx_request, _) = builder.build(browser.address()).await?;
            let tx_hash = browser.send_transaction_via_browser(tx_request).await?;

            let cast = CastTxSender::new(&provider);
            cast.print_tx_result(tx_hash, send_tx.cast_async, send_tx.confirmations, timeout).await
        // Case 3:
        // An option to use a local signer was provided.
        // If we cannot successfully instantiate a local signer, then we will assume we don't have
        // enough information to sign and we must bail.
        } else {
            let signer = send_tx.eth.wallet.signer().await?;
            let from = signer.address();

            tx::validate_from_address(send_tx.eth.wallet.from, from)?;

            let (tx_request, _) = builder.build(&signer).await?;

            let wallet = EthereumWallet::from(signer);
            let provider = AlloyProviderBuilder::<_, _, N>::default()
                .wallet(wallet)
                .connect_provider(&provider);

            cast_send(
                provider,
                tx_request,
                send_tx.cast_async,
                send_tx.sync,
                send_tx.confirmations,
                timeout,
            )
            .await
        }
    }
}

pub(crate) async fn cast_send<N: Network, P: Provider<N>>(
    provider: P,
    tx: N::TransactionRequest,
    cast_async: bool,
    sync: bool,
    confs: u64,
    timeout: u64,
) -> Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
    N::ReceiptResponse: UIfmt + UIfmtReceiptExt,
{
    let cast = CastTxSender::new(provider);

    if sync {
        // Send transaction and wait for receipt synchronously
        let receipt = cast.send_sync(tx).await?;
        sh_println!("{receipt}")?;
    } else {
        let pending_tx = cast.send(tx).await?;
        let tx_hash = *pending_tx.inner().tx_hash();
        cast.print_tx_result(tx_hash, cast_async, confs, timeout).await?;
    }

    Ok(())
}
