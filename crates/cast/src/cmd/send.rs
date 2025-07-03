use crate::{
    tx::{self, CastTxBuilder},
    Cast,
};
use alloy_ens::NameOrAddress;
use alloy_network::{AnyNetwork, EthereumWallet};
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use alloy_signer::Signer;
use clap::Parser;
use eyre::{eyre, Result};
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils,
    utils::LoadConfig,
};
use std::{path::PathBuf, str::FromStr};

/// Send a transaction to a contract or deploy a new contract.
///
/// Example:
///
/// cast send 0xAbC... "transfer(address,uint256)" 0x123... 100 --private-key &lt;KEY&gt; --rpc-url &lt;URL&gt;
/// cast send --create &lt;BYTECODE&gt; --private-key &lt;KEY&gt; --rpc-url &lt;URL&gt;
#[derive(Debug, Parser)]
#[command(
    about = "Send a transaction to a contract or deploy a new contract.",
    long_about = "Send a transaction to a contract or deploy a new contract.\n\
EXAMPLES:\n\
    cast send 0xAbC... 'transfer(address,uint256)' 0x123... 100 --private-key &lt;KEY&gt; --rpc-url &lt;URL&gt;\n\
    cast send --create &lt;BYTECODE&gt; --private-key &lt;KEY&gt; --rpc-url &lt;URL&gt;\n\
See more: https://book.getfoundry.sh/reference/cast/cast-send.html"
)]
pub struct SendTxArgs {
    /// Destination address of the transaction (contract or EOA).
    ///
    /// If not provided, you must use `cast send --create`.
    #[arg(value_name = "TO", value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// Function signature to call, e.g. `transfer(address,uint256)`.
    #[arg(value_name = "SIG")]
    sig: Option<String>,

    /// Arguments for the function call.
    #[arg(value_name = "ARGS")]
    args: Vec<String>,

    /// Only print the transaction hash and exit immediately.
    #[arg(id = "async", long = "async", alias = "cast-async", env = "CAST_ASYNC")]
    cast_async: bool,

    /// Number of confirmations to wait for the receipt.
    #[arg(long, default_value = "1", value_name = "NUM")]
    confirmations: u64,

    #[command(subcommand)]
    command: Option<SendTxSubcommands>,

    /// Use `eth_sendTransaction` with an unlocked account (requires --from or $ETH_FROM).
    #[arg(long, requires = "from")]
    unlocked: bool,

    /// Timeout (in seconds) for sending the transaction.
    #[arg(long, env = "ETH_TIMEOUT", value_name = "SECONDS")]
    pub timeout: Option<u64>,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,

    /// Path to a file containing blob data to be sent.
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
    /// Deploy raw contract bytecode as a new contract.
    #[command(name = "--create")]
    Create {
        /// Bytecode of the contract to deploy.
        #[arg(value_name = "BYTECODE")]
        code: String,

        /// Constructor signature, e.g. `constructor(uint256)`.
        #[arg(value_name = "SIG")]
        sig: Option<String>,

        /// Arguments for the constructor.
        #[arg(value_name = "ARGS")]
        args: Vec<String>,
    },
}

impl SendTxArgs {
    pub async fn run(self) -> eyre::Result<()> {
        let Self {
            eth,
            to,
            mut sig,
            cast_async,
            mut args,
            tx,
            confirmations,
            command,
            unlocked,
            path,
            timeout,
        } = self;

        let blob_data = if let Some(path) = path { Some(std::fs::read(path)?) } else { None };

        let code = if let Some(SendTxSubcommands::Create {
            code,
            sig: constructor_sig,
            args: constructor_args,
        }) = command
        {
            // ensure we don't violate settings for transactions that can't be CREATE: 7702 and 4844
            // which require mandatory target
            if to.is_none() && tx.auth.is_some() {
                return Err(eyre!("EIP-7702 transactions can't be CREATE transactions and require a destination address"));
            }
            // ensure we don't violate settings for transactions that can't be CREATE: 7702 and 4844
            // which require mandatory target
            if to.is_none() && blob_data.is_some() {
                return Err(eyre!("EIP-4844 transactions can't be CREATE transactions and require a destination address"));
            }

            sig = constructor_sig;
            args = constructor_args;
            Some(code)
        } else {
            None
        };

        let config = eth.load_config()?;
        let provider = utils::get_provider(&config)?;

        let builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;

        let timeout = timeout.unwrap_or(config.transaction_timeout);

        // Case 1:
        // Default to sending via eth_sendTransaction if the --unlocked flag is passed.
        // This should be the only way this RPC method is used as it requires a local node
        // or remote RPC with unlocked accounts.
        if unlocked {
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

            cast_send(provider, tx, cast_async, confirmations, timeout).await
        // Case 2:
        // An option to use a local signer was provided.
        // If we cannot successfully instantiate a local signer, then we will assume we don't have
        // enough information to sign and we must bail.
        } else {
            // Retrieve the signer, and bail if it can't be constructed.
            let signer = eth.wallet.signer().await?;
            let from = signer.address();

            tx::validate_from_address(eth.wallet.from, from)?;

            let (tx, _) = builder.build(&signer).await?;

            let wallet = EthereumWallet::from(signer);
            let provider = ProviderBuilder::<_, _, AnyNetwork>::default()
                .wallet(wallet)
                .connect_provider(&provider);

            cast_send(provider, tx, cast_async, confirmations, timeout).await
        }
    }
}

async fn cast_send<P: Provider<AnyNetwork>>(
    provider: P,
    tx: WithOtherFields<TransactionRequest>,
    cast_async: bool,
    confs: u64,
    timeout: u64,
) -> Result<()> {
    let cast = Cast::new(provider);
    let pending_tx = cast.send(tx).await?;

    let tx_hash = pending_tx.inner().tx_hash();

    if cast_async {
        sh_println!("{tx_hash:#x}")?;
    } else {
        let receipt =
            cast.receipt(format!("{tx_hash:#x}"), None, confs, Some(timeout), false).await?;
        sh_println!("{receipt}")?;
    }

    Ok(())
}
