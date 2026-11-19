//! OP-stack receipt construction for the Anvil block executor.

use alloy_consensus::{Eip658Value, Receipt, ReceiptWithBloom};
use alloy_primitives::Log;
use foundry_evm_hardforks::FoundryHardfork;
use foundry_primitives::FoundryReceiptEnvelope;
use op_alloy_consensus::{OpDepositReceipt, OpDepositReceiptWithBloom};

#[cfg(feature = "base")]
use foundry_evm_hardforks::BaseUpgrade;

#[cfg(feature = "optimism")]
use crate::eth::error::BlockchainError;
#[cfg(feature = "optimism")]
use alloy_consensus::Transaction;
#[cfg(feature = "optimism")]
use alloy_eips::Encodable2718;
#[cfg(feature = "optimism")]
use foundry_evm_hardforks::OpHardfork;
#[cfg(feature = "optimism")]
use foundry_primitives::FoundryTxEnvelope;

/// Returns the blob gas accounted for by an OP transaction under the active hardfork.
#[cfg(feature = "optimism")]
pub(crate) fn blob_gas_used<T: evm2::EvmTypes>(
    evm: &mut evm2::Evm<'_, T>,
    tx: &FoundryTxEnvelope,
    jovian: bool,
) -> Result<u64, BlockchainError> {
    if !jovian || tx.is_deposit() {
        return Ok(tx.blob_gas_used().unwrap_or_default());
    }

    let encoded =
        op_evm2::estimate_tx_compressed_size(tx.encoded_2718().as_ref()).saturating_div(1_000_000);
    let scalar = op_evm2::L1BlockInfo::fetch_da_footprint_gas_scalar(evm.state_mut())?;
    Ok(encoded.saturating_mul(u64::from(scalar)))
}

/// Builds an OP deposit receipt and derives its fork-specific metadata.
///
/// `depositor_nonce` is the sender nonce before the deposit executed. Without a configured
/// hardfork the receipt carries both fields, matching the network's default configuration.
pub(crate) fn build_deposit_receipt(
    success: bool,
    logs: Vec<Log>,
    hardfork: Option<FoundryHardfork>,
    depositor_nonce: u64,
    cumulative_gas_used: u64,
) -> FoundryReceiptEnvelope {
    let (deposit_nonce, deposit_receipt_version) = match hardfork {
        Some(hardfork) => deposit_metadata(hardfork, depositor_nonce),
        None => (Some(depositor_nonce), Some(1)),
    };
    let receipt =
        Receipt { status: Eip658Value::Eip658(success), cumulative_gas_used, logs }.with_bloom();
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
    {
        let _ = (hardfork, caller_nonce);
        (None, None)
    }
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
