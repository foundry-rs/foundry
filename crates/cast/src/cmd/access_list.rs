use crate::{
    Cast,
    rlp_converter::TryIntoRlpEncodable,
    tx::{CastTxBuilder, SenderKind},
};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_ens::NameOrAddress;
use alloy_network::{AnyNetwork, Network};
use alloy_rpc_types::BlockId;
use alloy_signer::Signature;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{RpcOpts, TransactionOpts},
    utils::LoadConfig,
};
use foundry_common::{
    fmt::{UIfmt, UIfmtHeaderExt, UIfmtSignatureExt},
    provider::ProviderBuilder,
};
use foundry_primitives::FoundryTransactionBuilder;
use foundry_wallets::WalletOpts;
use serde::Serialize;
use std::str::FromStr;
use tempo_alloy::TempoNetwork;

/// CLI arguments for `cast access-list`.
#[derive(Debug, Parser)]
pub struct AccessListArgs {
    /// The destination of the transaction.
    #[arg(
        value_name = "TO",
        value_parser = NameOrAddress::from_str
    )]
    to: Option<NameOrAddress>,

    /// The signature of the function to call.
    #[arg(value_name = "SIG")]
    sig: Option<String>,

    /// The arguments of the function to call.
    #[arg(value_name = "ARGS", allow_negative_numbers = true)]
    args: Vec<String>,

    /// The block height to query at.
    ///
    /// Can also be the tags earliest, finalized, safe, latest, or pending.
    #[arg(long, short = 'B')]
    block: Option<BlockId>,

    #[command(flatten)]
    tx: TransactionOpts,

    #[command(flatten)]
    rpc: RpcOpts,

    #[command(flatten)]
    wallet: WalletOpts,
}

impl AccessListArgs {
    pub async fn run(self) -> Result<()> {
        if self.tx.tempo.fee_token.is_some() || self.tx.tempo.sequence_key.is_some() {
            self.run_generic::<TempoNetwork>().await
        } else {
            self.run_generic::<AnyNetwork>().await
        }
    }

    pub async fn run_generic<N>(self) -> Result<()>
    where
        N: Network,
        N::TxEnvelope: From<Signed<N::UnsignedTx>> + Serialize + UIfmtSignatureExt,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: FoundryTransactionBuilder<N>,
        N::Header: TryIntoRlpEncodable,
        N::TransactionResponse: UIfmt,
        N::HeaderResponse: UIfmtHeaderExt,
        N::BlockResponse: UIfmt,
    {
        let Self { to, sig, args, tx, rpc, wallet, block } = self;

        let config = rpc.load_config()?;
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
        let sender = SenderKind::from_wallet_opts(wallet).await?;

        let (tx, _) = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(None, sig, args)
            .await?
            .build_raw(sender)
            .await?;

        let cast = Cast::new(&provider);

        let access_list: String = cast.access_list(&tx, block).await?;

        sh_println!("{access_list}")?;

        Ok(())
    }
}
