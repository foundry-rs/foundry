use super::auth::confirm_and_build;
use crate::tx::{CastTxBuilder, read_only_sender};
use alloy_ens::NameOrAddress;
use alloy_network::{Ethereum, Network};
use alloy_provider::Provider;
use alloy_rpc_types::BlockId;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    opts::{RpcOpts, TransactionOpts},
    utils::LoadConfig,
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder, shell};
use foundry_wallets::{BrowserWalletOpts, WalletOpts};
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

    /// Raw hex-encoded data for the transaction. Used instead of `SIG` and `ARGS`.
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

    /// Skip the EIP-7702 authorization disclosure confirmation.
    #[arg(long)]
    force: bool,

    #[command(flatten)]
    rpc: RpcOpts,

    #[command(flatten)]
    wallet: WalletOpts,

    #[command(flatten)]
    browser: BrowserWalletOpts,
}

impl AccessListArgs {
    pub async fn run(self) -> Result<()> {
        if self.tx.tempo.is_tempo() {
            self.run_with_network::<TempoNetwork>().await
        } else {
            self.run_with_network::<Ethereum>().await
        }
    }

    async fn run_with_network<N: Network + Unpin>(self) -> Result<()>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let Self { to, sig, args, data, tx, force, rpc, wallet, browser, block } = self;

        let config = rpc.load_config()?;
        let provider = ProviderBuilder::<N>::from_config(&config)?.build()?;
        let (sender, _) = read_only_sender::<N>(&browser, wallet).await?;

        let builder = CastTxBuilder::new(&provider, tx, &config)
            .await?
            .with_to(to)
            .await?
            .with_code_sig_and_args(None, data.or(sig), args)
            .await?
            .raw();
        let Some(tx) = confirm_and_build(builder, sender, force, None, true).await? else {
            return Ok(());
        };

        let access_list =
            provider.create_access_list(&tx).block_id(block.unwrap_or_default()).await?;
        let access_list = if shell::is_json() {
            serde_json::to_string(&access_list)?
        } else {
            let mut s =
                vec![format!("gas used: {}", access_list.gas_used), "access list:".to_string()];
            for al in access_list.access_list.0 {
                s.push(format!("- address: {}", al.address.to_checksum(None)));
                if !al.storage_keys.is_empty() {
                    s.push("  keys:".to_string());
                    for key in al.storage_keys {
                        s.push(format!("    {key:?}"));
                    }
                }
            }
            s.join("\n")
        };
        sh_println!("{access_list}")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

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
