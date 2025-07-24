use crate::EnvMut;
use alloy_chains::Chain;
use alloy_consensus::BlockHeader;
use alloy_hardforks::EthereumHardfork;
use alloy_json_abi::{Function, JsonAbi};
use alloy_network::{AnyTxEnvelope, TransactionResponse};
use alloy_primitives::{Address, B256, ChainId, Selector, TxKind, U256};
use alloy_provider::{Network, network::BlockResponse};
use alloy_rpc_types::{Transaction, TransactionRequest};
use foundry_config::NamedChain;
use revm::primitives::{
    eip4844::{BLOB_BASE_FEE_UPDATE_FRACTION_CANCUN, BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE},
    hardfork::SpecId,
};
pub use revm::state::EvmState as StateChangeset;

/// Hints to the compiler that this is a cold path, i.e. unlikely to be taken.
#[cold]
#[inline(always)]
pub fn cold_path() {
    // TODO: remove `#[cold]` and call `std::hint::cold_path` once stable.
}

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
            Moonbeam | Moonbase | Moonriver | MoonbeamDev | Rsk | RskTestnet => {
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

/// Derive the blob base fee update fraction based on the chain and timestamp by checking the
/// hardfork.
pub fn get_blob_base_fee_update_fraction(chain_id: ChainId, timestamp: u64) -> u64 {
    let hardfork = EthereumHardfork::from_chain_and_timestamp(Chain::from_id(chain_id), timestamp)
        .unwrap_or_default();

    if hardfork >= EthereumHardfork::Prague {
        BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE
    } else {
        BLOB_BASE_FEE_UPDATE_FRACTION_CANCUN
    }
}

/// Returns the blob base fee update fraction based on the spec id.
pub fn get_blob_base_fee_update_fraction_by_spec_id(spec: SpecId) -> u64 {
    if spec >= SpecId::PRAGUE {
        BLOB_BASE_FEE_UPDATE_FRACTION_PRAGUE
    } else {
        BLOB_BASE_FEE_UPDATE_FRACTION_CANCUN
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
    let from = tx.from();
    if let AnyTxEnvelope::Ethereum(tx) = &tx.inner.inner() {
        configure_tx_req_env(env, &tx.clone().into(), Some(from)).expect("cannot fail");
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
    // If no transaction type is provided, we need to infer it from the other fields.
    let tx_type = tx.transaction_type.unwrap_or_else(|| tx.minimal_tx_type() as u8);
    env.tx.tx_type = tx_type;

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
        ref authorization_list,
        transaction_type: _,
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

    // Type 1, EIP-2930
    env.tx.access_list = access_list.clone().unwrap_or_default();

    // Type 2, EIP-1559
    env.tx.gas_price = gas_price.or(max_fee_per_gas).unwrap_or_default();
    env.tx.gas_priority_fee = max_priority_fee_per_gas;

    // Type 3, EIP-4844
    env.tx.blob_hashes = blob_versioned_hashes.clone().unwrap_or_default();
    env.tx.max_fee_per_blob_gas = max_fee_per_blob_gas.unwrap_or_default();

    // Type 4, EIP-7702
    env.tx.set_signed_authorization(authorization_list.clone().unwrap_or_default());

    Ok(())
}
