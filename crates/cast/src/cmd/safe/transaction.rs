use crate::{
    tempo,
    tx::{CastTxBuilder, apply_poll_interval},
};
use alloy_consensus::{SignableTransaction, Signed};
use alloy_network::{Ethereum, EthereumWallet, Network, ReceiptResponse};
use alloy_primitives::{Address, B256, Bytes, hex};
use alloy_provider::{Provider, fillers::RecommendedFillers};
use alloy_rpc_types::Log;
use alloy_signer::{Signature, Signer};
use clap::Args;
use eyre::{Context, Result, ensure};
use foundry_cli::{
    opts::{EthereumOpts, RpcOpts, TransactionOpts},
    utils::{LoadConfig, resolve_lane},
};
use foundry_common::{FoundryTransactionBuilder, provider::ProviderBuilder};
use foundry_wallets::{WalletOpts, WalletSigner};
use serde::Serialize;
use std::time::Duration;
use tempo_alloy::TempoNetwork;

/// Options for sending the onchain transaction of `cast safe create` and `cast safe execute`.
#[derive(Args, Debug)]
pub(super) struct SafeSendOpts {
    #[command(flatten)]
    pub(super) rpc: Box<RpcOpts>,

    #[command(flatten)]
    wallet: Box<WalletOpts>,

    #[command(flatten)]
    tx: Box<TransactionOpts>,
}

pub(super) struct SafeSendResult {
    pub(super) tx_hash: B256,
    pub(super) logs: Vec<Log>,
}

impl SafeSendOpts {
    /// Sends a zero-value call of `data` to `to` and waits for its receipt.
    pub(super) async fn send(
        self,
        to: Address,
        data: Bytes,
        confirmations: u64,
        timeout: Option<u64>,
        poll_interval: Option<u64>,
    ) -> Result<SafeSendResult> {
        let Self { rpc, wallet, tx } = self;
        let eth = EthereumOpts { rpc: *rpc, wallet: *wallet, ..Default::default() };
        let (is_tempo, signer, access_key) =
            tempo::resolve_transaction_network_and_signer(&tx.tempo, &eth).await?;
        ensure!(
            access_key.is_none(),
            "Tempo Accounts sessions are not yet supported by `cast safe create` or `cast safe execute`"
        );
        let call = SafeCall { to, data, confirmations, timeout, poll_interval };
        if is_tempo {
            call.send::<TempoNetwork>(eth, *tx, signer).await
        } else {
            call.send::<Ethereum>(eth, *tx, signer).await
        }
    }
}

struct SafeCall {
    to: Address,
    data: Bytes,
    confirmations: u64,
    timeout: Option<u64>,
    poll_interval: Option<u64>,
}

impl SafeCall {
    async fn send<N>(
        self,
        eth: EthereumOpts,
        mut tx_opts: TransactionOpts,
        signer: Option<WalletSigner>,
    ) -> Result<SafeSendResult>
    where
        N: Network + RecommendedFillers,
        N::TxEnvelope: From<Signed<N::UnsignedTx>>,
        N::UnsignedTx: SignableTransaction<Signature>,
        N::TransactionRequest: Default + FoundryTransactionBuilder<N>,
        N::ReceiptResponse: Serialize,
    {
        ensure!(
            tx_opts.value.is_none_or(|value| value.is_zero()),
            "Safe outer transaction value must be zero"
        );
        ensure!(!tx_opts.blob, "blob transactions are not supported by `cast safe`");
        ensure!(
            tx_opts.auth.is_empty(),
            "EIP-7702 authorizations are not supported by `cast safe`"
        );
        ensure!(
            !tx_opts.tempo.has_sponsor_submission()
                && tx_opts.tempo.sponsor_url.is_none()
                && !tx_opts.tempo.print_sponsor_hash,
            "Tempo sponsorship is not yet supported by `cast safe create` or `cast safe execute`"
        );
        ensure!(
            tx_opts.tempo.session_id()?.is_none(),
            "Tempo Accounts sessions are not yet supported by `cast safe create` or `cast safe execute`"
        );

        let config = eth.rpc.load_config()?;
        let timeout = self.timeout.unwrap_or(config.transaction_timeout);
        resolve_lane(&mut tx_opts.tempo, &config.root)?;
        tempo::print_expires(tx_opts.tempo.resolve_expires())?;
        let signer = match signer {
            Some(signer) => signer,
            None => eth.wallet.signer().await?,
        };
        crate::tx::validate_from_address(eth.wallet.from, signer.address())?;
        let from = signer.address();
        let provider = ProviderBuilder::<N>::from_config(&config)?
            .build_with_wallet(EthereumWallet::from(signer))?;
        apply_poll_interval(&provider, self.poll_interval);
        let builder = CastTxBuilder::new(&provider, tx_opts, &config)
            .await?
            .with_to(Some(self.to.into()))
            .await?
            .with_code_sig_and_args(None, Some(hex::encode_prefixed(self.data)), Vec::new())
            .await?;
        let chain = builder.chain();
        let (mut request, _) = builder.build(from).await?;
        let fee_provider = (!config.eth_rpc_curl).then_some(&provider);
        tempo::resolve_and_print_fee_token(fee_provider, Some(chain), &mut request, Some(from))
            .await?;

        let receipt = provider
            .send_transaction(request)
            .await?
            .with_required_confirmations(self.confirmations)
            .with_timeout(Some(Duration::from_secs(timeout)))
            .get_receipt()
            .await?;
        ensure!(receipt.status(), "Safe transaction reverted");
        let tx_hash = receipt.transaction_hash();
        let receipt = serde_json::to_value(receipt)?;
        let logs = serde_json::from_value(receipt["logs"].clone())
            .wrap_err("invalid logs in transaction receipt")?;
        Ok(SafeSendResult { tx_hash, logs })
    }
}
