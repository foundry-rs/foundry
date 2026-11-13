use super::{
    contracts::{ISafe, ISafeLegacy},
    rpc_provider,
    service::{SafeServiceOpts, SafeTransaction},
    transaction::SafeSendOpts,
};
use alloy_primitives::{Address, B256};
use alloy_rpc_types::Log;
use alloy_sol_types::{SolCall, SolEvent};
use clap::Args;
use eyre::{Context, Result, ensure};
use foundry_cli::json::print_scalar;
use foundry_common::sh_status;

/// CLI arguments for `cast safe execute`.
#[derive(Args, Debug)]
pub struct ExecuteArgs {
    /// Safe account address.
    safe: Address,

    /// Safe transaction hash from the Transaction Service.
    safe_tx_hash: B256,

    /// Number of confirmations to wait for.
    #[arg(long, default_value = "1")]
    confirmations: u64,

    /// Timeout for execution confirmation, in seconds.
    #[arg(long, env = "ETH_TIMEOUT")]
    timeout: Option<u64>,

    /// Polling interval for the execution receipt, in seconds.
    #[arg(long, env = "ETH_POLL_INTERVAL")]
    poll_interval: Option<u64>,

    #[command(flatten)]
    service: Box<SafeServiceOpts>,

    #[command(flatten)]
    send: SafeSendOpts,
}

impl ExecuteArgs {
    pub(super) async fn run(self) -> Result<()> {
        let Self { safe, safe_tx_hash, confirmations, timeout, poll_interval, service, send } =
            self;
        let (provider, chain_id) = rpc_provider(&send.rpc).await?;
        let transaction = service.get_transaction(chain_id, "v2", safe_tx_hash).await?;
        ensure!(
            !transaction.is_executed && transaction.transaction_hash.is_none(),
            "Safe transaction has already been executed{}",
            transaction
                .transaction_hash
                .map(|hash| format!(" onchain as {hash}"))
                .unwrap_or_default()
        );

        transaction.verify_hash(safe, &provider, chain_id).await?;
        let transaction_nonce = SafeTransaction::number(&transaction.nonce, "nonce")?;
        let current_nonce = ISafe::new(safe, &provider)
            .nonce()
            .call()
            .await
            .wrap_err("failed to read Safe nonce")?;
        ensure!(
            transaction_nonce == current_nonce,
            "Safe transaction nonce {transaction_nonce} does not match current Safe nonce {current_nonce}"
        );
        transaction.show_transaction_summary(chain_id)?;
        let signatures = transaction.packed_signatures()?;

        sh_status!("Executing Safe transaction {safe_tx_hash}")?;
        let calldata = ISafe::execTransactionCall {
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
        let result = send
            .send(safe, calldata, confirmations, timeout, poll_interval)
            .await
            .wrap_err("failed to submit Safe transaction")?;
        ensure!(
            execution_succeeded(&result.logs, safe, safe_tx_hash)?,
            "Safe inner transaction failed"
        );
        print_scalar(result.tx_hash)
    }
}

/// Returns the outcome of the last `ExecutionSuccess`/`ExecutionFailure` event for `safe_tx_hash`.
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
            if let Ok(event) = ISafeLegacy::ExecutionSuccess::decode_log(&log.inner)
                && event.txHash == safe_tx_hash
            {
                return Some(true);
            }
            if let Ok(event) = ISafeLegacy::ExecutionFailure::decode_log(&log.inner)
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
    use alloy_primitives::{LogData, U256};
    use alloy_sol_types::SolValue;

    fn log(safe: Address, data: LogData) -> Log {
        Log { inner: alloy_primitives::Log { address: safe, data }, ..Default::default() }
    }

    #[test]
    fn uses_last_matching_safe_execution_event() {
        let safe = Address::repeat_byte(1);
        let hash = B256::repeat_byte(2);
        let success = || {
            log(
                safe,
                ISafe::ExecutionSuccess { txHash: hash, payment: U256::ZERO }.encode_log_data(),
            )
        };
        let failure = || {
            log(
                safe,
                ISafe::ExecutionFailure { txHash: hash, payment: U256::ZERO }.encode_log_data(),
            )
        };

        assert!(execution_succeeded(&[failure(), success()], safe, hash).unwrap());
        assert!(!execution_succeeded(&[success(), failure()], safe, hash).unwrap());
    }

    #[test]
    fn decodes_nonindexed_tx_hash_result_events() {
        let safe = Address::repeat_byte(1);
        let hash = B256::repeat_byte(2);
        // Safe v1.1.1 through v1.3.0 emit result events without indexed fields.
        let legacy = |topic0| {
            log(safe, LogData::new_unchecked(vec![topic0], (hash, U256::ZERO).abi_encode().into()))
        };

        assert!(
            execution_succeeded(&[legacy(ISafe::ExecutionSuccess::SIGNATURE_HASH)], safe, hash)
                .unwrap()
        );
        assert!(
            !execution_succeeded(&[legacy(ISafe::ExecutionFailure::SIGNATURE_HASH)], safe, hash)
                .unwrap()
        );
        assert!(
            execution_succeeded(
                &[legacy(ISafe::ExecutionSuccess::SIGNATURE_HASH)],
                safe,
                B256::ZERO
            )
            .is_err()
        );
    }
}
