use crate::tx;
use clap::Parser;
use ethers_core::types::NameOrAddress;
use ethers_middleware::MiddlewareBuilder;
use ethers_providers::Middleware;
use ethers_signers::Signer;
use eyre::Result;
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils,
};
use foundry_common::types::ToAlloy;
use foundry_config::Config;
use std::str::FromStr;

/// CLI arguments for `cast mktx`.
#[derive(Debug, Parser)]
pub struct MakeTxArgs {
    /// The destination of the transaction.
    ///
    /// If not provided, you must use `cast mktx --create`.
    #[arg(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// Reuse the latest nonce for the sender account.
    #[arg(long, conflicts_with = "nonce")]
    resend: bool,

    #[command(subcommand)]
    command: Option<MakeTxSubcommands>,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    eth: EthereumOpts,
}

#[derive(Debug, Parser)]
pub enum MakeTxSubcommands {
    /// Use to deploy raw contract bytecode.
    #[clap(name = "--create")]
    Create {
        /// The initialization bytecode of the contract to deploy.
        code: String,

        /// The signature of the constructor.
        sig: Option<String>,

        /// The constructor arguments.
        args: Vec<String>,
    },
}

impl MakeTxArgs {
    pub async fn run(self) -> Result<()> {
        let MakeTxArgs { to, mut sig, mut args, resend, command, mut tx, eth } = self;

        let code = if let Some(MakeTxSubcommands::Create {
            code,
            sig: constructor_sig,
            args: constructor_args,
        }) = command
        {
            sig = constructor_sig;
            args = constructor_args;
            Some(code)
        } else {
            None
        };

        tx::validate_to_address(&code, &to)?;

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain, &provider).await?;
        let api_key = config.get_etherscan_api_key(Some(chain));

        // Retrieve the signer, and bail if it can't be constructed.
        let signer = eth.wallet.signer().await?;
        let from = signer.address();

        tx::validate_from_address(eth.wallet.from, from.to_alloy())?;

        if resend {
            tx.nonce = Some(provider.get_transaction_count(from, None).await?.to_alloy());
        }

        let provider = provider.with_signer(signer);

        let (mut tx, _) =
            tx::build_tx(&provider, from, to, code, sig, args, tx, chain, api_key).await?;

        // Fill nonce, gas limit, gas price, and max priority fee per gas if needed
        provider.fill_transaction(&mut tx, None).await?;

        let signature = provider.sign_transaction(&tx, from).await?;
        let signed_tx = tx.rlp_signed(&signature);
        println!("{signed_tx}");

        Ok(())
    }
}
