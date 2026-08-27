use super::{
    contracts::ISafe,
    service::{SafeServiceOpts, SafeTransaction},
    transaction::send_safe_call,
};
use alloy_network::Ethereum;
use alloy_primitives::{Address, B256, Bytes};
use alloy_provider::Provider;
use alloy_rpc_types::Log;
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
    safe: Address,
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

    transaction.verify_hash(safe, &read_provider).await?;
    transaction.show_transaction_summary()?;

    let signatures = transaction.packed_signatures()?;

    sh_status!("Executing Safe transaction {safe_tx_hash}")?;
    let calldata: Bytes = ISafe::execTransactionCall {
        to: transaction.to,
        value: SafeTransaction::number(&transaction.value, "value")?,
        data: transaction.data.clone(),
        operation: transaction.operation,
        safeTxGas: SafeTransaction::number(&transaction.safe_tx_gas, "safeTxGas")?,
        baseGas: SafeTransaction::number(&transaction.base_gas, "baseGas")?,
        gasPrice: SafeTransaction::number(&transaction.gas_price, "gasPrice")?,
        gasToken: transaction.gas_token,
        refundReceiver: transaction.refund_receiver,
        signatures,
    }
    .abi_encode()
    .into();
    let result =
        send_safe_call(safe, calldata, confirmations, timeout, poll_interval, rpc, wallet, tx)
            .await
            .wrap_err("failed to submit Safe transaction")?;
    ensure!(
        execution_succeeded(&result.logs, safe, safe_tx_hash)?,
        "Safe inner transaction failed"
    );
    print_scalar(result.tx_hash)?;
    Ok(())
}

fn execution_succeeded(logs: &[Log], safe: Address, safe_tx_hash: B256) -> Result<bool> {
    logs.iter()
        .rev()
        .filter(|log| log.address() == safe)
        .find_map(|log| {
            if let Ok(event) = ISafe::ExecutionSuccess::decode_log(&log.inner)
                && event.txHash == safe_tx_hash
            {
                return Some(true);
            }
            if let Ok(event) = ISafe::ExecutionFailure::decode_log(&log.inner)
                && event.txHash == safe_tx_hash
            {
                return Some(false);
            }
            None
        })
        .ok_or_else(|| eyre::eyre!("Safe execution receipt did not emit a matching result event"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    fn success(safe: Address, hash: B256) -> Log {
        Log {
            inner: alloy_primitives::Log::new_from_event_unchecked(
                safe,
                ISafe::ExecutionSuccess { txHash: hash, payment: U256::ZERO },
            )
            .reserialize(),
            ..Default::default()
        }
    }

    fn failure(safe: Address, hash: B256) -> Log {
        Log {
            inner: alloy_primitives::Log::new_from_event_unchecked(
                safe,
                ISafe::ExecutionFailure { txHash: hash, payment: U256::ZERO },
            )
            .reserialize(),
            ..Default::default()
        }
    }

    #[test]
    fn uses_last_matching_safe_execution_event() {
        let safe = Address::repeat_byte(1);
        let hash = B256::repeat_byte(2);

        assert!(
            execution_succeeded(&[failure(safe, hash), success(safe, hash)], safe, hash).unwrap()
        );
        assert!(
            !execution_succeeded(&[success(safe, hash), failure(safe, hash)], safe, hash).unwrap()
        );
    }
}
