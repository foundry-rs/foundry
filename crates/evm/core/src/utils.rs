use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{FixedBytes, U256};
use alloy_rpc_types::{Block, Transaction};
use eyre::ContextCompat;
use foundry_config::NamedChain;
use revm::{
    db::WrapDatabaseRef,
    primitives::{SpecId, TransactTo},
};

pub use foundry_compilers::utils::RuntimeOrHandle;
pub use revm::primitives::State as StateChangeset;

pub use crate::ic::*;

/// Depending on the configured chain id and block number this should apply any specific changes
///
/// - checks for prevrandao mixhash after merge
/// - applies chain specifics: on Arbitrum `block.number` is the L1 block
/// Should be called with proper chain id (retrieved from provider if not provided).
pub fn apply_chain_and_block_specific_env_changes(env: &mut revm::primitives::Env, block: &Block) {
    if let Ok(chain) = NamedChain::try_from(env.cfg.chain_id) {
        let block_number = block.header.number.unwrap_or_default();

        match chain {
            NamedChain::Mainnet => {
                // after merge difficulty is supplanted with prevrandao EIP-4399
                if block_number.to::<u64>() >= 15_537_351u64 {
                    env.block.difficulty = env.block.prevrandao.unwrap_or_default().into();
                }

                return;
            }
            NamedChain::Arbitrum |
            NamedChain::ArbitrumGoerli |
            NamedChain::ArbitrumNova |
            NamedChain::ArbitrumTestnet => {
                // on arbitrum `block.number` is the L1 block which is included in the
                // `l1BlockNumber` field
                if let Some(l1_block_number) = block.other.get("l1BlockNumber").cloned() {
                    if let Ok(l1_block_number) = serde_json::from_value::<U256>(l1_block_number) {
                        env.block.number = l1_block_number;
                    }
                }
            }
            _ => {}
        }
    }

    // if difficulty is `0` we assume it's past merge
    if block.header.difficulty.is_zero() {
        env.block.difficulty = env.block.prevrandao.unwrap_or_default().into();
    }
}

/// Given an ABI and selector, it tries to find the respective function.
pub fn get_function(
    contract_name: &str,
    selector: &FixedBytes<4>,
    abi: &JsonAbi,
) -> eyre::Result<Function> {
    abi.functions()
        .find(|func| func.selector().as_slice() == selector.as_slice())
        .cloned()
        .wrap_err(format!("{contract_name} does not have the selector {selector:?}"))
}

/// Configures the env for the transaction
pub fn configure_tx_env(env: &mut revm::primitives::Env, tx: &Transaction) {
    env.tx.caller = tx.from;
    env.tx.gas_limit = tx.gas as u64;
    env.tx.gas_price = U256::from(tx.gas_price.unwrap_or_default());
    env.tx.gas_priority_fee = tx.max_priority_fee_per_gas.map(U256::from);
    env.tx.nonce = Some(tx.nonce);
    env.tx.access_list = tx
        .access_list
        .clone()
        .unwrap_or_default()
        .0
        .into_iter()
        .map(|item| {
            (
                item.address,
                item.storage_keys
                    .into_iter()
                    .map(|key| alloy_primitives::U256::from_be_bytes(key.0))
                    .collect(),
            )
        })
        .collect();
    env.tx.value = tx.value.to();
    env.tx.data = alloy_primitives::Bytes(tx.input.0.clone());
    env.tx.transact_to = tx.to.map(TransactTo::Call).unwrap_or_else(TransactTo::create)
}

/// Get the gas used, accounting for refunds
pub fn gas_used(spec: SpecId, spent: u64, refunded: u64) -> u64 {
    let refund_quotient = if SpecId::enabled(spec, SpecId::LONDON) { 5 } else { 2 };
    spent - (refunded).min(spent / refund_quotient)
}

/// Creates a new EVM with the given inspector.
pub fn new_evm_with_inspector<'a, DB, I>(
    db: DB,
    env: revm::primitives::EnvWithHandlerCfg,
    inspector: I,
) -> revm::Evm<'a, I, DB>
where
    DB: revm::Database,
    I: revm::Inspector<DB>,
{
    // NOTE: We could use `revm::Evm::builder()` here, but on the current patch it has some
    // performance issues.
    let revm::primitives::EnvWithHandlerCfg { env, handler_cfg } = env;
    let context = revm::Context::new(revm::EvmContext::new_with_env(db, env), inspector);
    let mut handler = revm::Handler::new(handler_cfg);
    handler.append_handler_register_plain(revm::inspector_handle_register);
    revm::Evm::new(context, handler)
}

/// Creates a new EVM with the given inspector and wraps the database in a `WrapDatabaseRef`.
pub fn new_evm_with_inspector_ref<'a, DB, I>(
    db: DB,
    env: revm::primitives::EnvWithHandlerCfg,
    inspector: I,
) -> revm::Evm<'a, I, WrapDatabaseRef<DB>>
where
    DB: revm::DatabaseRef,
    I: revm::Inspector<WrapDatabaseRef<DB>>,
{
    new_evm_with_inspector(WrapDatabaseRef(db), env, inspector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_evm() {
        let mut db = revm::db::EmptyDB::default();

        let env = Box::<revm::primitives::Env>::default();
        let spec = SpecId::LATEST;
        let handler_cfg = revm::primitives::HandlerCfg::new(spec);
        let cfg = revm::primitives::EnvWithHandlerCfg::new(env, handler_cfg);

        let mut inspector = revm::inspectors::NoOpInspector;

        let mut evm = new_evm_with_inspector(&mut db, cfg, &mut inspector);
        let result = evm.transact().unwrap();
        assert!(result.result.is_success());
    }
}
