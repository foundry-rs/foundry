use crate::{
    Cast,
    tx::{CastTxBuilder, SenderKind},
};
use alloy_ens::NameOrAddress;
use alloy_network::{AnyNetwork, Network};
use alloy_rpc_types::BlockId;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{RpcOpts, TransactionOpts},
    utils::LoadConfig,
};
use foundry_common::provider::ProviderBuilder;
use foundry_primitives::FoundryTransactionBuilder;
use foundry_wallets::WalletOpts;
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

    /// Raw hex-encoded data for the transaction. Used instead of \[SIG\] and \[ARGS\].
    #[arg(
        long,
        conflicts_with_all = &["sig", "args"]
    )]
    data: Option<String>,

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
        if self.tx.tempo.is_tempo() {
            self.run_with_network::<TempoNetwork>().await
        } else {
            self.run_with_network::<AnyNetwork>().await
        }
    }

    pub async fn run_with_network<N: Network + Unpin>(self) -> Result<()>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let Self { to, mut sig, args, data, tx, rpc, wallet, block } = self;

        if let Some(data) = data {
            sig = Some(data);
        }

        let config = rpc.load_config()?;
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
        let sender = SenderKind::from_wallet_opts(wallet).await?;

        let (tx, _) = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(None, sig, args)
            .await?
            .raw()
            .build(sender)
            .await?;

        let access_list: String = Cast::new(&provider).access_list(&tx, block).await?;

        sh_println!("{access_list}")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex;
    use clap::error::ErrorKind;

    #[test]
    fn can_parse_access_list_data() {
        let data = hex::encode("hello");
        let args = AccessListArgs::parse_from(["foundry-cli", "--data", data.as_str()]);
        assert_eq!(args.data, Some(data));

        let data = hex::encode_prefixed("hello");
        let args = AccessListArgs::parse_from(["foundry-cli", "--data", data.as_str()]);
        assert_eq!(args.data, Some(data));
    }

    #[test]
    fn data_conflicts_with_sig_and_args() {
        let err = AccessListArgs::try_parse_from([
            "foundry-cli",
            "0x0000000000000000000000000000000000000001",
            "transfer(address,uint256)",
            "0x0000000000000000000000000000000000000002",
            "1",
            "--data",
            "0x1234",
        ])
        .unwrap_err();

        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }
}
