use cast::TxBuilder;
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
    #[clap(value_parser = NameOrAddress::from_str)]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    sig: Option<String>,

    /// The arguments of the function to call.
    args: Vec<String>,

    /// Reuse the latest nonce for the sender account.
    #[clap(long, conflicts_with = "nonce")]
    resend: bool,

    #[clap(subcommand)]
    command: Option<MakeTxSubcommands>,

    #[clap(flatten)]
    tx: TransactionOpts,

    #[clap(flatten)]
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

        // ensure mandatory fields are provided
        if code.is_none() && to.is_none() {
            eyre::bail!("Must specify a recipient address or contract code to deploy");
        }

        let config = Config::from(&eth);
        let provider = utils::get_provider(&config)?;
        let chain = utils::get_chain(config.chain, &provider).await?;
        let api_key = config.get_etherscan_api_key(Some(chain));

        // Retrieve the signer, and bail if it can't be constructed.
        let signer = eth.wallet.signer(chain.id()).await?;
        let from = signer.address();

        // prevent misconfigured hwlib from sending a transaction that defies
        // user-specified --from
        if let Some(specified_from) = eth.wallet.from {
            if specified_from != from.to_alloy() {
                eyre::bail!(
                    "\
The specified sender via CLI/env vars does not match the sender configured via
the hardware wallet's HD Path.
Please use the `--hd-path <PATH>` parameter to specify the BIP32 Path which
corresponds to the sender, or let foundry automatically detect it by not specifying any sender address."
                )
            }
        }

        if resend {
            tx.nonce = Some(provider.get_transaction_count(from, None).await?.to_alloy());
        }

        let provider = provider.with_signer(signer);

        let params = sig.as_deref().map(|sig| (sig, args));
        let mut builder = TxBuilder::new(&provider, from, to, chain, tx.legacy).await?;
        builder
            .etherscan_api_key(api_key)
            .gas(tx.gas_limit)
            .gas_price(tx.gas_price)
            .priority_gas_price(tx.priority_gas_price)
            .value(tx.value)
            .nonce(tx.nonce);

        if let Some(code) = code {
            let mut data = hex::decode(code)?;

            if let Some((sig, args)) = params {
                let (mut sigdata, _) = builder.create_args(sig, args).await?;
                data.append(&mut sigdata);
            }

            builder.set_data(data);
        } else {
            builder.args(params).await?;
        }
        let (mut tx, _) = builder.build();

        // Fill nonce, gas limit, gas price, and max priority fee per gas if needed
        provider.fill_transaction(&mut tx, None).await?;

        let signature = provider.sign_transaction(&tx, from).await?;
        let signed_tx = tx.rlp_signed(&signature);
        println!("{signed_tx}");

        Ok(())
    }
}
