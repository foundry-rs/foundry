//! OP-stack receipt construction for the Anvil block executor.

#[cfg(feature = "optimism")]
use alloy_consensus::Transaction;
use alloy_consensus::{Eip658Value, Receipt, ReceiptWithBloom};
#[cfg(feature = "optimism")]
use alloy_eips::Encodable2718;
use alloy_primitives::{Address, Log};
use foundry_evm::hardfork::FoundryHardfork;
#[cfg(feature = "optimism")]
use foundry_evm::hardfork::OpHardfork;
#[cfg(feature = "base")]
use foundry_evm::hardforks::BaseUpgrade;
use foundry_primitives::FoundryReceiptEnvelope;
#[cfg(feature = "optimism")]
use foundry_primitives::FoundryTxEnvelope;
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom};
#[cfg(feature = "optimism")]
use op_revm::{L1BlockInfo, estimate_tx_compressed_size};
#[cfg(feature = "optimism")]
use revm::Database;
use revm::{context_interface::result::ExecutionResult, state::EvmState};

#[cfg(feature = "optimism")]
use super::AnvilBlockExecutor;

#[cfg(feature = "optimism")]
impl<E> AnvilBlockExecutor<E> {
    /// Configures OP-specific block accounting without changing the shared constructor.
    pub(crate) fn set_optimism_hardfork(&mut self, hardfork: FoundryHardfork) {
        self.optimism_jovian = OpHardfork::from(hardfork) >= OpHardfork::Jovian;
    }
}

/// Returns the blob gas accounted for by an OP transaction under the active hardfork.
#[cfg(feature = "optimism")]
pub(crate) fn blob_gas_used<DB: Database>(
    db: &mut DB,
    tx: &FoundryTxEnvelope,
    jovian: bool,
) -> Result<u64, alloy_evm::block::BlockExecutionError> {
    if !jovian || matches!(tx, FoundryTxEnvelope::Deposit(_)) {
        return Ok(tx.blob_gas_used().unwrap_or_default());
    }

    let encoded = estimate_tx_compressed_size(tx.encoded_2718().as_ref()).saturating_div(1_000_000);
    let scalar = L1BlockInfo::fetch_da_footprint_gas_scalar(db)
        .map_err(alloy_evm::block::BlockExecutionError::other)?;
    Ok(encoded.saturating_mul(u64::from(scalar)))
}

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
    let (deposit_nonce, deposit_receipt_version) = deposit_metadata(hardfork, caller_nonce);
    let receipt =
        Receipt { status: Eip658Value::Eip658(result.is_success()), cumulative_gas_used, logs }
            .with_bloom();
    wrap_deposit_receipt(receipt, deposit_nonce, deposit_receipt_version)
}

/// Resolves the deposit nonce and receipt version active at `hardfork`.
///
/// Base is an OP-stack chain, so it gates the same two fields on its own upgrade names.
fn deposit_metadata(hardfork: FoundryHardfork, caller_nonce: u64) -> (Option<u64>, Option<u64>) {
    #[cfg(feature = "base")]
    if matches!(hardfork, FoundryHardfork::Base(_)) {
        let upgrade = BaseUpgrade::from(hardfork);
        return (
            (upgrade >= BaseUpgrade::Regolith).then_some(caller_nonce),
            (upgrade >= BaseUpgrade::Canyon).then_some(1),
        );
    }
    #[cfg(feature = "optimism")]
    {
        let hardfork = OpHardfork::from(hardfork);
        (
            (hardfork >= OpHardfork::Regolith).then_some(caller_nonce),
            (hardfork >= OpHardfork::Canyon).then_some(1),
        )
    }
    #[cfg(not(feature = "optimism"))]
    (None, None)
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
