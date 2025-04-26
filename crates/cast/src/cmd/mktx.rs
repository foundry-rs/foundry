use crate::tx::{self, CastTxBuilder};
use alloy_network::{eip2718::Encodable2718, EthereumWallet, TransactionBuilder};
use alloy_primitives::{hex, Address, Bytes, U256, U64};
use alloy_rlp::{Decodable, RlpDecodable, RlpEncodable};
use alloy_signer::Signer;
use clap::Parser;
use eyre::{OptionExt, Result};
use foundry_cli::{
    opts::{EthereumOpts, TransactionOpts},
    utils::{get_provider, LoadConfig},
};
use foundry_common::ens::NameOrAddress;
use std::{path::PathBuf, str::FromStr};

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

    #[command(subcommand)]
    command: Option<MakeTxSubcommands>,

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

    #[command(flatten)]
    eth: EthereumOpts,

    /// Generate a raw RLP-encoded unsigned transaction.
    ///
    /// Relaxes the wallet requirement.
    #[arg(long, requires = "from")]
    raw_unsigned: bool,

    #[arg(long, value_name = "V")]
    v: Option<u64>,

    #[arg(long, value_name = "R")]
    r: Option<String>,

    #[arg(long, value_name = "S")]
    s: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, RlpDecodable, RlpEncodable)]
pub struct UnsignedTransaction {
    pub nonce: U256,
    pub gas_price: U256,
    pub gas_limit: U256,
    pub to: Address,
    pub value: U256,
    pub data: Bytes,
}

#[derive(Debug, Parser)]
pub enum MakeTxSubcommands {
    /// Use to deploy raw contract bytecode.
    #[command(name = "--create")]
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
        let Self { mut to, mut sig, mut args, command, tx, path, eth, raw_unsigned, v, r, s } =
            self;

        if !raw_unsigned && to.is_none() && !args.is_empty() {
            let to_str = args.remove(0);
            to = Some(NameOrAddress::from_str(&to_str)?);
        }

        let blob_data = if let Some(path) = path { Some(std::fs::read(path)?) } else { None };

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

        let config = eth.load_config()?;

        let provider = get_provider(&config)?;

        let tx_builder = CastTxBuilder::new(provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(code, sig, args)
            .await?
            .with_blob_data(blob_data)?;

        if raw_unsigned {
            let from = eth.wallet.from.ok_or_eyre("missing `--from` address")?;
            let raw_unsigned_hex = tx_builder.build_unsigned_raw(from).await?;

            if let (Some(v), Some(r_hex), Some(s_hex)) = (v, r.as_ref(), s.as_ref()) {
                let raw_unsigned_bytes = hex::decode(raw_unsigned_hex.trim_start_matches("0x"))?;
                let mut buf = raw_unsigned_bytes.as_slice();
                let unsigned_tx: UnsignedTransaction = UnsignedTransaction::decode(&mut buf)?;

                let r_bytes = hex::decode(r_hex.trim_start_matches("0x"))?;
                let s_bytes = hex::decode(s_hex.trim_start_matches("0x"))?;

                if r_bytes.len() != 32 || s_bytes.len() != 32 {
                    eyre::bail!("r and s must be 32 bytes each");
                }

                let r = U256::from_be_slice(&r_bytes);
                let s = U256::from_be_slice(&s_bytes);
                let v = U64::from(v);

                let mut out = Vec::new();
                let fields: &[&dyn alloy_rlp::Encodable] = &[
                    &unsigned_tx.nonce,
                    &unsigned_tx.gas_price,
                    &unsigned_tx.gas_limit,
                    &unsigned_tx.to,
                    &unsigned_tx.value,
                    &unsigned_tx.data,
                    &v,
                    &r,
                    &s,
                ];

                alloy_rlp::encode_list::<&dyn alloy_rlp::Encodable, dyn alloy_rlp::Encodable>(
                    fields, &mut out,
                );

                sh_println!("0x{}", hex::encode(out))?;
            } else {
                sh_println!("{}", raw_unsigned_hex)?;
            }

            return Ok(());
        }

        let signer = eth.wallet.signer().await?;
        let from = signer.address();

        tx::validate_from_address(eth.wallet.from, from)?;

        let (tx, _) = tx_builder.build(&signer).await?;

        let tx = tx.build(&EthereumWallet::new(signer)).await?;

        let signed_tx = hex::encode(tx.encoded_2718());
        sh_println!("0x{signed_tx}")?;

        Ok(())
    }
}
