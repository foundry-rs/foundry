//! OP-stack receipt construction for the Anvil block executor.

use alloy_consensus::{Eip658Value, Receipt, ReceiptWithBloom};
use alloy_primitives::{Address, Log};
use foundry_evm::hardfork::{FoundryHardfork, OpHardfork};
use foundry_primitives::FoundryReceiptEnvelope;
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom};
use revm::{context_interface::result::ExecutionResult, state::EvmState};

/// Builds a mined OP deposit receipt and derives its fork-specific metadata.
pub(crate) fn build_mined_deposit_receipt<H>(
    result: ExecutionResult<H>,
    state: &EvmState,
    sender: Address,
    cumulative_gas_used: u64,
) -> FoundryReceiptEnvelope {
    let deposit_nonce = state.get(&sender).map(|account| account.info.nonce);
    build_deposit_receipt(result, cumulative_gas_used, deposit_nonce, deposit_nonce.map(|_| 1))
}

/// Builds an RPC-simulated OP deposit receipt and derives its fork-specific metadata.
pub(crate) fn build_simulated_deposit_receipt<H>(
    hardfork: FoundryHardfork,
    caller_nonce: u64,
    result: &ExecutionResult<H>,
    logs: Vec<Log>,
    cumulative_gas_used: u64,
) -> FoundryReceiptEnvelope {
    let hardfork = OpHardfork::from(hardfork);
    let deposit_nonce = (hardfork >= OpHardfork::Regolith).then_some(caller_nonce);
    let deposit_receipt_version = (hardfork >= OpHardfork::Canyon).then_some(1);
    let receipt =
        Receipt { status: Eip658Value::Eip658(result.is_success()), cumulative_gas_used, logs }
            .with_bloom();
    wrap_deposit_receipt(receipt, deposit_nonce, deposit_receipt_version)
}

fn build_deposit_receipt<H>(
    result: ExecutionResult<H>,
    cumulative_gas_used: u64,
    deposit_nonce: Option<u64>,
    deposit_receipt_version: Option<u64>,
) -> FoundryReceiptEnvelope {
    let receipt = Receipt {
        status: Eip658Value::Eip658(result.is_success()),
        cumulative_gas_used,
        logs: result.into_logs(),
    }
    .with_bloom();
    wrap_deposit_receipt(receipt, deposit_nonce, deposit_receipt_version)
}

fn wrap_deposit_receipt(
    receipt: ReceiptWithBloom<Receipt>,
    deposit_nonce: Option<u64>,
    deposit_receipt_version: Option<u64>,
) -> FoundryReceiptEnvelope {
    FoundryReceiptEnvelope::Deposit(OpDepositReceiptWithBloom {
        receipt: OpDepositReceipt {
            inner: receipt.receipt,
            deposit_nonce,
            deposit_receipt_version,
        },
        logs_bloom: receipt.logs_bloom,
    })
}
