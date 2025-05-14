use alloy_consensus::BlockHeader;
use alloy_json_abi::{Function, JsonAbi};
use alloy_network::{AnyTxEnvelope, TransactionResponse};
use alloy_primitives::{Address, Selector, TxKind, B256, U256};
use alloy_provider::{network::BlockResponse, Network};
use alloy_rpc_types::{AccessList, Transaction, TransactionRequest};
use eyre::ensure;
use foundry_common::is_impersonated_tx;
use foundry_config::NamedChain;
pub use revm::state::EvmState as StateChangeset;
use revm::{context::TransactionType, primitives::hardfork::SpecId};

use crate::EnvMut;

/// Depending on the configured chain id and block number this should apply any specific changes
///
/// - checks for prevrandao mixhash after merge
/// - applies chain specifics: on Arbitrum `block.number` is the L1 block
///
/// Should be called with proper chain id (retrieved from provider if not provided).
pub fn apply_chain_and_block_specific_env_changes<N: Network>(
    env: EnvMut<'_>,
    block: &N::BlockResponse,
) {
    use NamedChain::*;

    if let Ok(chain) = NamedChain::try_from(env.cfg.chain_id) {
        let block_number = block.header().number();

        match chain {
            Mainnet => {
                // after merge difficulty is supplanted with prevrandao EIP-4399
                if block_number >= 15_537_351u64 {
                    env.block.difficulty = env.block.prevrandao.unwrap_or_default().into();
                }

                return;
            }
            BinanceSmartChain | BinanceSmartChainTestnet => {
                // https://github.com/foundry-rs/foundry/issues/9942
                // As far as observed from the source code of bnb-chain/bsc, the `difficulty` field
                // is still in use and returned by the corresponding opcode but `prevrandao`
                // (`mixHash`) is always zero, even though bsc adopts the newer EVM
                // specification. This will confuse revm and causes emulation
                // failure.
                env.block.prevrandao = Some(env.block.difficulty.into());
                return;
            }
            Moonbeam | Moonbase | Moonriver | MoonbeamDev | Rsk => {
                if env.block.prevrandao.is_none() {
                    // <https://github.com/foundry-rs/foundry/issues/4232>
                    env.block.prevrandao = Some(B256::random());
                }
            }
            c if c.is_arbitrum() => {
                // on arbitrum `block.number` is the L1 block which is included in the
                // `l1BlockNumber` field
                if let Some(l1_block_number) = block
                    .other_fields()
                    .and_then(|other| other.get("l1BlockNumber").cloned())
                    .and_then(|l1_block_number| {
                        serde_json::from_value::<U256>(l1_block_number).ok()
                    })
                {
                    env.block.number = l1_block_number.to();
                }
            }
            _ => {}
        }
    }

    // if difficulty is `0` we assume it's past merge
    if block.header().difficulty().is_zero() {
        env.block.difficulty = env.block.prevrandao.unwrap_or_default().into();
    }
}

/// Given an ABI and selector, it tries to find the respective function.
pub fn get_function<'a>(
    contract_name: &str,
    selector: Selector,
    abi: &'a JsonAbi,
) -> eyre::Result<&'a Function> {
    abi.functions()
        .find(|func| func.selector() == selector)
        .ok_or_else(|| eyre::eyre!("{contract_name} does not have the selector {selector}"))
}

/// Configures the env for the given RPC transaction.
/// Accounts for an impersonated transaction by resetting the `env.tx.caller` field to `tx.from`.
pub fn configure_tx_env(env: &mut EnvMut<'_>, tx: &Transaction<AnyTxEnvelope>) {
    let impersonated_from = is_impersonated_tx(&tx.inner).then_some(tx.from());
    if let AnyTxEnvelope::Ethereum(tx) = &tx.inner.inner() {
        configure_tx_req_env(env, &tx.clone().into(), impersonated_from).expect("cannot fail");
    }
}

/// Configures the env for the given RPC transaction request.
/// `impersonated_from` is the address of the impersonated account. This helps account for an
/// impersonated transaction by resetting the `env.tx.caller` field to `impersonated_from`.
pub fn configure_tx_req_env(
    env: &mut EnvMut<'_>,
    tx: &TransactionRequest,
    impersonated_from: Option<Address>,
) -> eyre::Result<()> {
    let TransactionRequest {
        nonce,
        from,
        to,
        value,
        gas_price,
        gas,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        max_fee_per_blob_gas,
        ref input,
        chain_id,
        ref blob_versioned_hashes,
        ref access_list,
        transaction_type,
        ref authorization_list,
        sidecar: _,
    } = *tx;

    // If no `to` field then set create kind: https://eips.ethereum.org/EIPS/eip-2470#deployment-transaction
    env.tx.kind = to.unwrap_or(TxKind::Create);
    // If the transaction is impersonated, we need to set the caller to the from
    // address Ref: https://github.com/foundry-rs/foundry/issues/9541
    env.tx.caller =
        impersonated_from.unwrap_or(from.ok_or_else(|| eyre::eyre!("missing `from` field"))?);
    env.tx.gas_limit = gas.ok_or_else(|| eyre::eyre!("missing `gas` field"))?;
    env.tx.nonce = nonce.unwrap_or_default();
    env.tx.value = value.unwrap_or_default();
    env.tx.data = input.input().cloned().unwrap_or_default();
    env.tx.chain_id = chain_id;

    // Type 0, Legacy, default transaction type.
    let mut tx_type = TransactionType::Legacy;

    // Type 1, EIP-2930, derived from existence of `access_list`.
    if let Some(access_list) = access_list {
        // If the transaction type is legacy we need to upgrade it to EIP-2930 transaction type
        // in order to use access lists. Other transaction types support access lists
        // themselves.
        tx_type = TransactionType::Eip2930;
        env.tx.access_list = access_list.clone();
    } else {
        env.tx.access_list = AccessList::default();
    }

    // Type 2, EIP-1559, derived from existence of `Some(max_fee_per_gas)`.
    if max_fee_per_gas.is_some() {
        tx_type = TransactionType::Eip1559;
    }
    env.tx.gas_price = gas_price.or(max_fee_per_gas).unwrap_or_default();
    env.tx.gas_priority_fee = max_priority_fee_per_gas;

    // Type 3, EIP-4844, derived from existence of `Some(max_fee_per_blob_gas)`.
    if max_fee_per_blob_gas.is_some() {
        ensure!(to.is_some(), "EIP-4844 transactions must have a `to` field set");
        tx_type = TransactionType::Eip4844;
    }
    env.tx.blob_hashes = blob_versioned_hashes.clone().unwrap_or_default();
    env.tx.max_fee_per_blob_gas = max_fee_per_blob_gas.unwrap_or_default();

    // Type 4, EIP-7702, derived from existence of `authorization_list`.
    if let Some(authorization_list) = authorization_list {
        // If the transaction has an authorization list, we need to set the transaction type to
        // EIP-7702.
        ensure!(to.is_some(), "EIP-7702 transactions must have a `to` field set");
        tx_type = TransactionType::Eip7702;
        env.tx.set_signed_authorization(authorization_list.clone());
    }

    // If the transaction type is not already set we use the derived transaction type.
    if let Some(transaction_type) = transaction_type {
        env.tx.tx_type = transaction_type;
    } else {
        env.tx.tx_type = tx_type as u8;
    }

    Ok(())
}

/// Get the gas used, accounting for refunds
pub fn gas_used(spec: SpecId, spent: u64, refunded: u64) -> u64 {
    let refund_quotient = if SpecId::is_enabled_in(spec, SpecId::LONDON) { 5 } else { 2 };
    spent - (refunded).min(spent / refund_quotient)
}
