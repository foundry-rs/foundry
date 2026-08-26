use crate::tempo;
use alloy_consensus::{SignableTransaction, Signed};
use alloy_network::{Ethereum, EthereumWallet, Network, ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, Bytes};
use alloy_provider::{Provider, fillers::RecommendedFillers};
use alloy_rpc_types::Log;
use alloy_signer::{Signature, Signer};
use eyre::{Context, Result, ensure};
use foundry_cli::{
    opts::{EthereumOpts, RpcOpts, TransactionOpts},
    utils::{LoadConfig, get_chain, resolve_lane},
};
use foundry_common::{
    FoundryTransactionBuilder,
    provider::ProviderBuilder,
    tempo::{maybe_print_fee_token, resolve_and_set_fee_token},
};
use foundry_wallets::WalletOpts;
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;
use tempo_alloy::TempoNetwork;

pub(super) struct SafeSendResult {
    pub(super) tx_hash: B256,
    pub(super) receipt: Value,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn send_safe_call(
    to: Address,
    data: Bytes,
    confirmations: u64,
    timeout: Option<u64>,
    poll_interval: Option<u64>,
    rpc: RpcOpts,
    wallet: WalletOpts,
    tx: TransactionOpts,
) -> Result<SafeSendResult> {
    let eth = EthereumOpts { rpc: rpc.clone(), wallet: wallet.clone(), ..Default::default() };
    let (is_tempo, signer, access_key) =
        tempo::resolve_transaction_network_and_signer(&tx.tempo, &eth).await?;
    ensure!(
        access_key.is_none(),
        "Tempo Accounts sessions are not yet supported by `cast safe create` or `cast safe execute`"
    );
    if is_tempo {
        send_safe_call_generic::<TempoNetwork>(
            to,
            data,
            confirmations,
            timeout,
            poll_interval,
            rpc,
            wallet,
            tx,
            signer,
        )
        .await
    } else {
        send_safe_call_generic::<Ethereum>(
            to,
            data,
            confirmations,
            timeout,
            poll_interval,
            rpc,
            wallet,
            tx,
            signer,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_safe_call_generic<N>(
    to: Address,
    data: Bytes,
    confirmations: u64,
    timeout: Option<u64>,
    poll_interval: Option<u64>,
    rpc: RpcOpts,
    wallet_opts: WalletOpts,
    mut tx_opts: TransactionOpts,
    signer: Option<foundry_wallets::WalletSigner>,
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
    ensure!(tx_opts.auth.is_empty(), "EIP-7702 authorizations are not supported by `cast safe`");
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

    let config = rpc.load_config()?;
    let timeout = timeout.unwrap_or(config.transaction_timeout);
    let _resolved_lane = resolve_lane(&mut tx_opts.tempo, &config.root)?;
    let expires_at = tx_opts.tempo.resolve_expires();
    tempo::print_expires(expires_at)?;
    let signer = match signer {
        Some(signer) => signer,
        None => wallet_opts.signer().await?,
    };
    crate::tx::validate_from_address(wallet_opts.from, signer.address())?;
    let from = signer.address();
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::<N>::from_config(&config)?.build_with_wallet(wallet)?;
    if let Some(interval) = poll_interval {
        provider.client().set_poll_interval(Duration::from_secs(interval));
    }
    let chain = get_chain(config.chain, &provider).await?;
    let mut request = N::TransactionRequest::default().with_to(to).with_input(data);
    tx_opts.apply::<N>(&mut request, chain.is_legacy());
    let fee_token = resolve_and_set_fee_token(
        (!config.eth_rpc_curl).then_some(&provider),
        Some(chain),
        &mut request,
        Some(from),
    )
    .await?;
    maybe_print_fee_token((!config.eth_rpc_curl).then_some(&provider), fee_token).await?;

    let receipt = provider
        .send_transaction(request)
        .await?
        .with_required_confirmations(confirmations)
        .with_timeout(Some(Duration::from_secs(timeout)))
        .get_receipt()
        .await?;
    ensure!(receipt.status(), "Safe transaction reverted");
    Ok(SafeSendResult {
        tx_hash: receipt.transaction_hash(),
        receipt: serde_json::to_value(receipt)?,
    })
}

pub(super) fn receipt_logs(receipt: &Value) -> Result<Vec<Log>> {
    serde_json::from_value(receipt.get("logs").cloned().unwrap_or_else(|| json!([])))
        .wrap_err("invalid logs in transaction receipt")
}
