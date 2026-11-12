use super::{
    contracts::{ISafe, ISafeV1_3_0},
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

        transaction.verify_hash(safe, &provider).await?;
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
        transaction.show_transaction_summary()?;
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
            if let Ok(event) = ISafeV1_3_0::ExecutionSuccess::decode_log(&log.inner)
                && event.txHash == safe_tx_hash
            {
                return Some(true);
            }
            if let Ok(event) = ISafeV1_3_0::ExecutionFailure::decode_log(&log.inner)
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
    use std::slice;

    fn log(safe: Address, data: LogData) -> Log {
        Log { inner: alloy_primitives::Log { address: safe, data }, ..Default::default() }
    }

    #[test]
    fn uses_last_matching_safe_execution_event() {
        let safe = Address::repeat_byte(1);
        let hash = B256::repeat_byte(2);
        for (first, first_expected) in execution_events(safe, hash) {
            for (last, expected) in execution_events(safe, hash) {
                if first_expected != expected {
                    assert_eq!(
                        execution_succeeded(&[first.clone(), last], safe, hash).unwrap(),
                        expected
                    );
                }
            }
        }
    }

    #[test]
    fn ignores_unrelated_execution_events() {
        let safe = Address::repeat_byte(1);
        let hash = B256::repeat_byte(2);
        let [indexed_success, indexed_failure, nonindexed_success, nonindexed_failure] =
            execution_events(safe, hash);
        // Pair each unrelated event with the opposite outcome in the same layout.
        let matching =
            [&indexed_failure, &indexed_success, &nonindexed_failure, &nonindexed_success];
        let wrong_hash = execution_events(safe, B256::repeat_byte(4));
        let [wrong_emitter, ..] = execution_events(Address::repeat_byte(3), hash);
        for ((unrelated, _), (matching, expected)) in
            wrong_hash.iter().zip(matching).chain([(&wrong_emitter, &indexed_failure)])
        {
            assert!(execution_succeeded(slice::from_ref(unrelated), safe, hash).is_err());
            assert_eq!(
                execution_succeeded(&[matching.clone(), unrelated.clone()], safe, hash).unwrap(),
                *expected
            );
        }
    }

    #[test]
    fn rejects_malformed_execution_events() {
        let safe = Address::repeat_byte(1);
        let hash = B256::repeat_byte(2);
        for (event, expected) in execution_events(safe, hash) {
            // Clearing the signature makes success and failure identical within each layout.
            if expected {
                let mut wrong_signature = event.clone();
                wrong_signature.inner.data.topics_mut()[0] = B256::ZERO;
                assert!(execution_succeeded(&[wrong_signature], safe, hash).is_err());
            }

            let mut truncated = event;
            let data = &mut truncated.inner.data.data;
            *data = data.slice(..data.len() - 1);
            assert!(execution_succeeded(&[truncated], safe, hash).is_err());
        }
    }

    fn execution_events(safe: Address, hash: B256) -> [(Log, bool); 4] {
        [
            (
                log(
                    safe,
                    ISafe::ExecutionSuccess { txHash: hash, payment: U256::ZERO }.encode_log_data(),
                ),
                true,
            ),
            (
                log(
                    safe,
                    ISafe::ExecutionFailure { txHash: hash, payment: U256::ZERO }.encode_log_data(),
                ),
                false,
            ),
            (
                log(
                    safe,
                    ISafeV1_3_0::ExecutionSuccess { txHash: hash, payment: U256::ZERO }
                        .encode_log_data(),
                ),
                true,
            ),
            (
                log(
                    safe,
                    ISafeV1_3_0::ExecutionFailure { txHash: hash, payment: U256::ZERO }
                        .encode_log_data(),
                ),
                false,
            ),
        ]
    }
}
