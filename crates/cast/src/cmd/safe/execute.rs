use super::{
    contracts::ISafe,
    service::{SafeServiceOpts, SafeTransaction},
    transaction::{receipt_logs, send_safe_call},
};
use alloy_network::Ethereum;
use alloy_primitives::{B256, Bytes};
use alloy_provider::Provider;
use alloy_sol_types::{SolCall, SolEvent};
use eyre::{Context, Result, ensure};
use foundry_cli::{
    json::print_scalar,
    opts::{RpcOpts, TransactionOpts},
    utils::LoadConfig,
};
use foundry_common::{provider::ProviderBuilder, sh_status};
use foundry_wallets::WalletOpts;
use reqwest::Method;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    safe_tx_hash: B256,
    confirmations: u64,
    timeout: Option<u64>,
    poll_interval: Option<u64>,
    service: SafeServiceOpts,
    rpc: RpcOpts,
    wallet: WalletOpts,
    tx: TransactionOpts,
) -> Result<()> {
    let config = rpc.load_config()?;
    let timeout = timeout.unwrap_or(config.transaction_timeout);
    let read_provider = ProviderBuilder::<Ethereum>::from_config(&config)?.build()?;
    let chain_id = read_provider.get_chain_id().await?;
    let url = service.endpoint(chain_id, &format!("v2/multisig-transactions/{safe_tx_hash}/"))?;
    let transaction: SafeTransaction = service.response(service.request(Method::GET, url)).await?;
    ensure!(
        transaction.safe_tx_hash == safe_tx_hash,
        "Transaction Service returned a different Safe transaction hash"
    );
    ensure!(
        !transaction.is_executed && transaction.transaction_hash.is_none(),
        "Safe transaction has already been executed{}",
        transaction.transaction_hash.map(|hash| format!(" onchain as {hash}")).unwrap_or_default()
    );

    transaction.verify_hash(&read_provider).await?;
    transaction.show_signing_summary()?;

    let signatures = transaction.packed_signatures()?;
    let safe_address = transaction.safe()?;
    let threshold = ISafe::new(safe_address, &read_provider).getThreshold().call().await?;
    ensure!(
        signatures.len() / 65 >= threshold.to::<usize>(),
        "Safe transaction has {} signatures but requires {threshold}",
        signatures.len() / 65
    );

    sh_status!("Executing Safe transaction {safe_tx_hash}")?;
    let calldata: Bytes = ISafe::execTransactionCall {
        to: transaction.to()?,
        value: SafeTransaction::number(&transaction.value, "value")?,
        data: transaction.data.clone(),
        operation: transaction.operation,
        safeTxGas: SafeTransaction::number(&transaction.safe_tx_gas, "safeTxGas")?,
        baseGas: SafeTransaction::number(&transaction.base_gas, "baseGas")?,
        gasPrice: SafeTransaction::number(&transaction.gas_price, "gasPrice")?,
        gasToken: transaction.gas_token()?,
        refundReceiver: transaction.refund_receiver()?,
        signatures,
    }
    .abi_encode()
    .into();
    let result = send_safe_call(
        safe_address,
        calldata,
        confirmations,
        Some(timeout),
        poll_interval,
        rpc,
        wallet,
        tx,
    )
    .await
    .wrap_err("failed to submit Safe transaction")?;
    let logs = receipt_logs(&result.receipt)?;
    let failed = logs
        .iter()
        .filter(|log| log.address() == safe_address)
        .any(|log| ISafe::ExecutionFailure::decode_log(&log.inner).is_ok());
    ensure!(!failed, "Safe inner transaction failed");
    let succeeded = logs
        .iter()
        .filter(|log| log.address() == safe_address)
        .any(|log| ISafe::ExecutionSuccess::decode_log(&log.inner).is_ok());
    ensure!(succeeded, "Safe execution receipt did not emit ExecutionSuccess");
    print_scalar(result.tx_hash)?;
    Ok(())
}
