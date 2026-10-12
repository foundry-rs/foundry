//! Cast's local Ethereum execution through EVM2.

use crate::{debug::handle_traces, traces::TraceKind};
use alloy_consensus::{TypedTransaction, transaction::Recovered};
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{Address, U256, map::AddressHashMap};
use alloy_rpc_types::{
    BlockId, BlockOverrides, TransactionInput, TransactionRequest, state::StateOverride,
};
use evm2::{EvmFeatures, ExecutionConfig, ethereum::TxEnvelope};
use evm2_inspectors::tracing::{TracingInspector, TracingInspectorConfig};
use eyre::ensure;
use foundry_cli::utils::TraceResult;
use foundry_common::FoundryTransactionBuilder;
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{Chain, Config};
use foundry_evm::{
    core::ethereum::{EthereumEnv, EthereumFork},
    ethereum::Executor,
    hardforks::ethereum_spec_from_evm_version,
    opts::EvmOpts,
    traces::{SparsedTraceArena, TraceContext},
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn call<N: Network>(
    request: N::TransactionRequest,
    from: Address,
    mut config: Box<Config>,
    opts: EvmOpts,
    block: Option<BlockId>,
    state_overrides: Option<StateOverride>,
    block_overrides: Option<BlockOverrides>,
    version: Option<EvmVersion>,
    with_local_artifacts: bool,
    debug: bool,
) -> eyre::Result<()>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    ensure!(
        !debug && !config.tracing.decode_internal,
        "native EVM2 opcode debugging is not migrated yet"
    );
    let mut fork = EthereumFork::open(&config, &opts, block).await?;
    let mut env = fork.env;
    if let Some(version) = version {
        let block = env.block;
        let gas_price = env.gas_price;
        let chain_id = env.version.chain_id;
        env = EthereumEnv::local(ethereum_spec_from_evm_version(version), &opts);
        env.block = block;
        env.gas_price = gas_price;
        env.version.chain_id = chain_id;
        fork.hardfork = None;
    }
    env.version.features.remove(EvmFeatures::BLOCK_GAS_LIMIT_CHECK);
    env.version.tx_gas_limit_cap = u64::MAX;
    // Like eth_call, honor requested fees without requiring the caller to afford them.
    env.version.features.remove(EvmFeatures::FEE_CHARGE);
    if request.max_fee_per_gas().or(request.gas_price()).unwrap_or_default() == 0
        && request.max_priority_fee_per_gas().unwrap_or_default() == 0
    {
        env.block.basefee = U256::ZERO;
    }
    env.block.gas_limit = U256::from(request.gas_limit().unwrap_or(opts.gas_limit()));
    if let Some(overrides) = block_overrides {
        if let Some(number) = overrides.number {
            env.block.number = U256::from(number);
        }
        if let Some(time) = overrides.time {
            env.block.timestamp = U256::from(time);
        }
    }
    let tx = transaction::<N>(&request, from, &env, opts.gas_limit())?;
    let mut executor = Executor::new(
        fork.database,
        env.spec,
        ExecutionConfig::for_spec_and_version(env.spec, env.version),
        env.block,
    );
    if let Some(overrides) = state_overrides {
        executor.apply_state_overrides(overrides)?;
    }
    let inspector = tracing_inspector(config.tracing.verbosity);
    let (result, inspector) = executor.inspect_call(&tx, inspector);
    let result = result?;
    let trace = trace_result(&result, &inspector, TraceKind::Execution);
    let contracts_bytecode = if with_local_artifacts {
        let mut code = inspector
            .traces()
            .nodes()
            .iter()
            .filter(|node| node.trace.success && node.trace.kind.is_any_create())
            .map(|node| (node.trace.address, node.trace.output.clone()))
            .collect::<AddressHashMap<_>>();
        for address in crate::cmd::run::trace_addresses(&trace) {
            if code.contains_key(&address) {
                continue;
            }
            let bytecode = executor.code(address)?;
            if !bytecode.is_empty() {
                code.insert(address, bytecode.original_bytes());
            }
        }
        code
    } else {
        Default::default()
    };
    let context = fork.fork.context();
    config.labels.extend(foundry_evm_networks::resolved_precompile_labels(fork.hardfork));
    handle_traces(
        trace,
        &config,
        TraceContext::new(Chain::from_id(context.source_chain_id), opts.networks, fork.hardfork),
        &contracts_bytecode,
        &config.tracing,
        with_local_artifacts,
        false,
    )
    .await
}

pub(super) fn trace_result(
    result: &evm2::evm::TxResult<foundry_evm::core::ethereum::FoundryEvmTypes>,
    inspector: &TracingInspector,
    kind: TraceKind,
) -> TraceResult {
    TraceResult {
        success: result.status,
        traces: Some(vec![(
            kind,
            SparsedTraceArena {
                arena: foundry_evm::traces::ethereum::native_arena(inspector).arena,
                ignored: Default::default(),
                diagnostics: Default::default(),
            },
        )]),
        gas_used: result.tx_gas_used(),
    }
}

fn transaction<N: Network>(
    request: &N::TransactionRequest,
    from: Address,
    env: &EthereumEnv,
    gas_limit: u64,
) -> eyre::Result<Recovered<TxEnvelope>>
where
    N::TransactionRequest: FoundryTransactionBuilder<N>,
{
    let dynamic = request.max_fee_per_gas().is_some()
        || request.max_priority_fee_per_gas().is_some()
        || request.blob_versioned_hashes().is_some()
        || request.authorization_list().is_some();
    let request = TransactionRequest {
        from: Some(from),
        to: request.kind(),
        value: request.value(),
        input: TransactionInput::new(request.input().cloned().unwrap_or_default()),
        nonce: Some(request.nonce().unwrap_or_default()),
        gas: Some(request.gas_limit().unwrap_or(gas_limit)),
        chain_id: Some(request.chain_id().unwrap_or(env.version.chain_id)),
        gas_price: (!dynamic).then_some(request.gas_price().unwrap_or_default()),
        max_fee_per_gas: dynamic.then_some(request.max_fee_per_gas().unwrap_or_default()),
        max_priority_fee_per_gas: dynamic
            .then_some(request.max_priority_fee_per_gas().unwrap_or_default()),
        access_list: request.access_list().cloned(),
        authorization_list: request.authorization_list().cloned(),
        max_fee_per_blob_gas: request.max_fee_per_blob_gas(),
        blob_versioned_hashes: request.blob_versioned_hashes().map(<[_]>::to_vec),
        ..Default::default()
    };
    let tx = match request
        .build_consensus_tx()
        .map_err(|error| eyre::eyre!("invalid local transaction request: {error:?}"))?
    {
        TypedTransaction::Legacy(tx) => TxEnvelope::Legacy(tx),
        TypedTransaction::Eip2930(tx) => TxEnvelope::Eip2930(tx),
        TypedTransaction::Eip1559(tx) => TxEnvelope::Eip1559(tx),
        TypedTransaction::Eip4844(tx) => TxEnvelope::Eip4844(tx),
        TypedTransaction::Eip7702(tx) => TxEnvelope::Eip7702(tx.into()),
    };
    Ok(Recovered::new_unchecked(tx, from))
}

pub(super) fn tracing_inspector(verbosity: u8) -> TracingInspector {
    TracingInspector::new(
        TracingInspectorConfig::default_parity()
            .set_record_inputs(true)
            .set_record_logs(true)
            .set_steps_and_state_diffs(verbosity > 4),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::Transaction;

    #[test]
    fn local_transaction_preserves_requested_fees() {
        let opts = EvmOpts::default();
        let env = EthereumEnv::local(evm2::SpecId::CANCUN, &opts);
        let from = Address::with_last_byte(1);
        let request =
            TransactionRequest::default().with_to(Address::with_last_byte(2)).with_gas_price(7);
        let tx = transaction::<alloy_network::Ethereum>(&request, from, &env, 100_000).unwrap();
        assert_eq!(tx.inner().gas_price(), Some(7));
        let request = TransactionRequest::default()
            .with_to(Address::with_last_byte(2))
            .with_max_fee_per_gas(11)
            .with_max_priority_fee_per_gas(3);
        let tx = transaction::<alloy_network::Ethereum>(&request, from, &env, 100_000).unwrap();
        assert_eq!(tx.inner().max_fee_per_gas(), 11);
        assert_eq!(tx.inner().max_priority_fee_per_gas(), Some(3));
    }
}
