//! `eth_simulateV1` with anvil's semantics on top of reth's block builder.
//!
//! Reth's own handler differs from anvil in the block sequence it fills in, the request gas
//! budget, the error codes and messages, the transfer logs (reth journals them, so they reach the
//! receipts and the bloom), the warm addresses after precompile moves, and the state roots. This
//! module keeps reth's execution and block assembly, and puts anvil's rules around them.

use crate::{
    api::{CallBatch, NonceLane},
    evm::AnvilNextBlockEnv,
    fork::ForkInfo,
};
use alloy_consensus::{BlockHeader, transaction::TxHashRef};
use alloy_eips::{BlockId, BlockNumberOrTag, eip2718::WithEncoded, eip4844::DATA_GAS_PER_BLOB};
use alloy_evm::{
    Evm,
    block::{BlockExecutionError, BlockExecutor, BlockValidationError, TxResult},
    env::BlockEnvironment,
    overrides::{OverrideBlockHashes, apply_block_overrides, apply_state_overrides},
    precompiles::{MovePrecompileError, PrecompilesMap},
};
use alloy_primitives::{Address, B256, Bytes, U256, address, b256, map::AddressSet};
use alloy_rpc_types_eth::{
    BlockOverrides, Log, TransactionRequest,
    simulate::{MAX_SIMULATE_BLOCKS, SimBlock, SimulatePayload, SimulatedBlock},
    state::{AccountOverride, StateOverride},
};
use alloy_serde::WithOtherFields;
use jsonrpsee::types::ErrorObject;
use reth_ethereum::{
    chainspec::{ChainSpecProvider, EthChainSpec, EthereumHardforks},
    evm::{
        primitives::{
            ConfigureEvm, HaltReasonFor,
            execute::{BlockBuilder, BlockBuilderOutcome},
        },
        revm::{
            database::StateProviderDatabase,
            db::{State, bal::BalState},
        },
    },
    primitives::{BlockBody as _, NodePrimitives, Recovered},
    rpc::eth::{
        EthApiError,
        error::ToRpcError,
        simulate::{self as reth_simulate, EthSimulateError},
    },
    storage::{StateProvider, noop::NoopProvider},
};
use reth_rpc_eth_api::{
    AsEthApiError, FromEthApiError, FromEvmError, RpcBlock, RpcConvert, RpcTxReq, helpers::EthCall,
};
use revm::{
    Database, DatabaseCommit, DatabaseRef, Inspector,
    context::{
        Block, ContextTr, JournalTr,
        result::{ExecutionResult, InvalidTransaction},
    },
    context_interface::Cfg,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, CreateScheme, Interpreter,
    },
    primitives::Log as PrimitiveLog,
    state::{Account, AccountStatus, EvmState},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt, sync::Arc};

/// The gas budget of one `eth_simulateV1` request, as anvil enforces it.
const SIMULATE_GAS_CAP: u64 = 50_000_000;

/// The seconds between simulated blocks when the node has no block interval configured.
pub(crate) const DEFAULT_BLOCK_INTERVAL_SECS: u64 = 12;

/// The emitter of the transfer logs `traceTransfers` adds to the response.
const TRANSFER_LOG_EMITTER: Address = address!("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");

/// The topic of `Transfer(address,address,uint256)`.
const TRANSFER_EVENT_TOPIC: B256 =
    b256!("0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");

/// An `eth_simulateV1` error with anvil's code and message.
#[derive(Debug)]
struct SimulateRpcError {
    code: i32,
    message: String,
}

impl fmt::Display for SimulateRpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SimulateRpcError {}

impl ToRpcError for SimulateRpcError {
    fn to_rpc_error(&self) -> ErrorObject<'static> {
        ErrorObject::owned(self.code, self.message.clone(), None::<()>)
    }
}

/// Builds an `eth_simulateV1` error with the given code and message.
fn simulate_error(code: i32, message: impl Into<String>) -> EthApiError {
    EthApiError::other(SimulateRpcError { code, message: message.into() })
}

/// Reports a blob fee cap below the block's blob base fee with revm's message, as anvil does.
fn blob_fee_error(error: BlockExecutionError) -> EthApiError {
    if let BlockExecutionError::Validation(BlockValidationError::InvalidTx {
        error: invalid, ..
    }) = &error
        && let Some(invalid) = invalid.as_invalid_tx_err()
        && matches!(invalid, InvalidTransaction::BlobGasPriceGreaterThanMax { .. })
    {
        return simulate_error(
            -32003,
            "Block `blob_gas_price` is greater than tx-specified `max_fee_per_blob_gas`",
        );
    }
    error.into()
}

/// The error anvil reports for a base block that does not exist.
fn header_not_found() -> EthApiError {
    simulate_error(-32000, "header not found")
}

/// Executes `eth_simulateV1` as anvil does.
///
/// `interval` is the seconds between simulated blocks without a `time` override, `base_fee` the
/// node's base fee override for the first block, and `compute_state_root` whether the blocks get
/// real state roots; fork databases are partial, so their blocks keep a zero root. Simulations on
/// blocks before the fork block are forwarded to the fork endpoint.
pub(crate) async fn simulate_v1<Eth, Net: Send + Sync + 'static>(
    eth: &Eth,
    payload: SimulatePayload<RpcTxReq<Eth::NetworkTypes>>,
    block: Option<BlockId>,
    interval: u64,
    base_fee: Option<u64>,
    compute_state_root: bool,
    fork: Option<Arc<dyn ForkInfo>>,
) -> Result<Vec<SimulatedBlock<WithOtherFields<RpcBlock<Eth::NetworkTypes>>>>, Eth::Error>
where
    Eth: EthCall + Clone + 'static,
    RpcTxReq<Eth::NetworkTypes>: CallBatch<Net>,
    <Eth::Evm as ConfigureEvm>::NextBlockEnvCtx: AnvilNextBlockEnv<Net>,
{
    let SimulatePayload {
        block_state_calls,
        trace_transfers,
        validation,
        return_full_transactions,
    } = payload;
    if block_state_calls.is_empty() {
        return Err(Eth::Error::from_eth_err(EthApiError::InvalidParams("empty input".into())));
    }
    if block_state_calls.len() > MAX_SIMULATE_BLOCKS as usize {
        return Err(Eth::Error::from_eth_err(simulate_error(-38026, "too many blocks")));
    }
    for call in block_state_calls.iter().flat_map(|block| &block.calls) {
        validate_request(call.as_ref()).map_err(Eth::Error::from_eth_err)?;
    }

    let block = block.unwrap_or_default();
    let base = eth.recovered_block(block).await?;

    // Blocks before the fork block live on the fork endpoint, which simulates them. A hash
    // selects the endpoint's block, which may differ from the local block at that height.
    let remote = match (&fork, &base) {
        (Some(fork), Some(base)) => base.header().number() < fork.block_number(),
        (Some(_), None) => matches!(block, BlockId::Hash(_)),
        (None, _) => false,
    };
    if remote && let Some(fork) = fork {
        let (id, number, timestamp) = match (block, base) {
            (BlockId::Hash(hash), _) => {
                let header = fork
                    .forward_json::<Option<alloy_rpc_types_eth::Block>>(
                        "eth_getBlockByHash",
                        json!([hash.block_hash, false]),
                    )
                    .map_err(|error| Eth::Error::from_eth_err(EthApiError::other(error)))?
                    .ok_or_else(|| Eth::Error::from_eth_err(header_not_found()))?
                    .header;
                (BlockId::Hash(hash), header.number(), header.timestamp())
            }
            // A number or tag selects the endpoint's canonical block, which the local cache may
            // know under another hash.
            (_, Some(base)) => {
                let number = base.header().number();
                let header = fork
                    .forward_json::<Option<alloy_rpc_types_eth::Block>>(
                        "eth_getBlockByNumber",
                        json!([BlockNumberOrTag::Number(number), false]),
                    )
                    .map_err(|error| Eth::Error::from_eth_err(EthApiError::other(error)))?
                    .ok_or_else(|| Eth::Error::from_eth_err(header_not_found()))?
                    .header;
                (BlockId::number(number), number, header.timestamp())
            }
            (_, None) => return Err(Eth::Error::from_eth_err(header_not_found())),
        };
        let block_state_calls = sanitize_blocks(block_state_calls, number, timestamp, interval)
            .map_err(Eth::Error::from_eth_err)?;
        let payload = SimulatePayload {
            block_state_calls,
            trace_transfers,
            validation,
            return_full_transactions,
        };
        return fork
            .forward_json("eth_simulateV1", json!([payload, id]))
            .map_err(|error| Eth::Error::from_eth_err(EthApiError::other(error)));
    }

    let base = base.ok_or_else(|| Eth::Error::from_eth_err(header_not_found()))?;
    let parent = base.clone_sealed_header();
    let block_state_calls =
        sanitize_blocks(block_state_calls, parent.number(), parent.timestamp(), interval)
            .map_err(Eth::Error::from_eth_err)?;

    let permit = eth
        .acquire_owned_blocking_io()
        .await
        .map_err(|_| Eth::Error::from_eth_err(EthApiError::InternalEthError))?;

    eth.spawn_with_state_at_block(block, move |this, db| {
        let _permit = permit;
        let state_provider = db.database.into_inner();
        let mut db = State::builder()
            .with_database(StateProviderDatabase::new(&state_provider))
            .with_bundle_update()
            .build();
        let mut parent = parent;
        let mut base_fee = base_fee;
        let mut budget = SIMULATE_GAS_CAP;
        let chain_spec = this.provider().chain_spec();
        let chain_id = chain_spec.chain_id();
        let mut blocks = Vec::with_capacity(block_state_calls.len());

        for block in block_state_calls {
            let SimBlock { block_overrides, state_overrides, calls } = block;
            let overrides = block_overrides.unwrap_or_default();
            let time =
                overrides.time.unwrap_or_else(|| parent.timestamp().saturating_add(interval));
            // The node's base fee override applies to the first block only; the others derive
            // their base fee from the simulated block before them.
            let block_base_fee = base_fee.take().unwrap_or_else(|| {
                parent
                    .next_block_base_fee(chain_spec.base_fee_params_at_timestamp(time))
                    .unwrap_or_default()
            });

            let mut attributes = this
                .pending_env_builder()
                .pending_env_attributes(&parent, Some(&overrides))
                .map_err(Eth::Error::from_eth_err)?;
            attributes.set_timestamp(time);
            attributes.set_prev_randao(B256::ZERO);
            if let Some(coinbase) = overrides.coinbase {
                attributes.set_suggested_fee_recipient(coinbase);
            }
            if let Some(gas_limit) = overrides.gas_limit {
                attributes.set_gas_limit(gas_limit);
            }
            let mut evm_env =
                this.evm_config().next_evm_env(&parent, &attributes).map_err(|error| {
                    Eth::Error::from_eth_err(EthApiError::EvmCustom(error.to_string()))
                })?;

            // Always disable EIP-3607.
            evm_env.cfg_env.disable_eip3607 = true;
            // EIP-7825's transaction gas cap is only active with Amsterdam's gas accounting.
            if !evm_env.cfg_env.is_amsterdam_eip8037_enabled() {
                evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);
            }
            if !validation {
                evm_env.cfg_env.disable_nonce_check = true;
                evm_env.cfg_env.disable_base_fee = true;
            }
            let block_env = evm_env.block_env.inner_mut();
            block_env.basefee = if validation { block_base_fee } else { 0 };
            block_env.prevrandao = Some(B256::ZERO);
            if !chain_spec.is_paris_active_at_block(block_env.number.saturating_to()) {
                block_env.difficulty = parent.difficulty();
            }

            // Block hash overrides are scoped to their block; remember what they replace.
            let replaced_hashes = overrides
                .block_hash
                .iter()
                .flatten()
                .map(|(number, _)| (*number, db.block_hashes.get(*number)))
                .collect::<Vec<_>>();
            apply_block_overrides(overrides, &mut db, evm_env.block_env.inner_mut());
            let is_amsterdam = chain_spec
                .is_amsterdam_active_at_timestamp(evm_env.block_env.timestamp().saturating_to());

            let ctx =
                this.evm_config().context_for_next_block(&parent, attributes).map_err(|error| {
                    Eth::Error::from_eth_err(EthApiError::EvmCustom(error.to_string()))
                })?;
            let evm = this.evm_config().evm_with_env_and_inspector(
                &mut db,
                evm_env,
                SimulationInspector::new(trace_transfers),
            );
            let mut builder = this.evm_config().create_block_builder(evm, &parent, ctx);

            if let Some(overrides) = state_overrides {
                let warm = apply_precompile_moves(&overrides, builder.evm_mut().precompiles_mut())
                    .map_err(Eth::Error::from_eth_err)?;
                builder.evm_mut().inspector_mut().warm_precompiles = warm;
                let overrides = overrides
                    .into_iter()
                    .filter(|(_, account)| overrides_state(account))
                    .collect::<StateOverride>();
                apply_state_overrides(overrides, builder.evm_mut().db_mut())
                    .map_err(Eth::Error::from_eth_err)?;
            }
            // Overrides define the starting state, so only execution contributes BAL writes.
            if is_amsterdam {
                builder.evm_mut().db_mut().bal_state = BalState::new().with_bal_builder();
            }

            let (outcome, calls) = execute_calls::<_, _, Net>(
                builder,
                &*state_provider,
                calls,
                &mut budget,
                chain_id,
                validation,
                chain_spec
                    .blob_params_at_timestamp(time)
                    .map_or(u64::MAX, |params| params.max_blob_gas_per_block()),
                compute_state_root,
                this.converter(),
            )
            .map_err(|error| match error.as_simulate_error() {
                Some(error) => EthApiError::other(error),
                None => error,
            })
            .map_err(Eth::Error::from_eth_err)?;

            let header = outcome.block.clone_sealed_header();
            for (number, hash) in replaced_hashes {
                let hash = match hash {
                    Some(hash) => Some(hash),
                    None => db.database.block_hash_ref(number).ok(),
                };
                if let Some(hash) = hash {
                    db.override_block_hashes(BTreeMap::from([(number, hash)]));
                }
            }
            db.override_block_hashes(BTreeMap::from([(header.number(), header.hash())]));
            parent = header;

            let block = build_block::<Eth::Error, _>(
                outcome,
                calls,
                return_full_transactions,
                this.converter(),
            )?;
            let inner = match &fork {
                Some(fork) => fork.with_block_fields(block.inner),
                None => WithOtherFields::new(block.inner),
            };
            blocks.push(SimulatedBlock { inner, calls: block.calls });
        }

        Ok(blocks)
    })
    .await
}

/// Rejects a request with conflicting fields, as every call and send entry point does.
pub(crate) fn validate_request(request: &TransactionRequest) -> Result<(), EthApiError> {
    request.input.clone().try_into_unique_input()?;
    if request.gas_price.is_some()
        && (request.max_fee_per_gas.is_some() || request.max_priority_fee_per_gas.is_some())
    {
        return Err(EthApiError::ConflictingFeeFieldsInRequest);
    }
    Ok(())
}

/// Runs a blob call without a blob fee cap at a zero blob base fee, as geth's `eth_call` does,
/// unless the block overrides set a blob base fee.
pub(crate) fn with_zero_blob_base_fee(
    request: &TransactionRequest,
    block_overrides: Option<Box<BlockOverrides>>,
) -> Option<Box<BlockOverrides>> {
    let blob = request.sidecar.is_some()
        || request.blob_versioned_hashes.as_ref().is_some_and(|hashes| !hashes.is_empty());
    if !blob || request.max_fee_per_blob_gas.is_some_and(|cap| cap != 0) {
        return block_overrides;
    }
    let mut block_overrides = block_overrides.unwrap_or_default();
    block_overrides.blob_base_fee.get_or_insert(U256::ZERO);
    Some(block_overrides)
}

/// Fills in the block numbers and timestamps of the simulated blocks, and the empty blocks in the
/// gaps between them, as anvil does.
fn sanitize_blocks<T>(
    blocks: Vec<SimBlock<T>>,
    base_number: u64,
    base_timestamp: u64,
    interval: u64,
) -> Result<Vec<SimBlock<T>>, EthApiError> {
    let interval = interval.max(1);
    let mut sanitized = Vec::with_capacity(blocks.len());
    let mut previous_number = base_number;
    let mut previous_timestamp = base_timestamp;

    for mut block in blocks {
        let mut overrides = block.block_overrides.take().unwrap_or_default();
        let default_number = previous_number.checked_add(1).ok_or_else(|| {
            simulate_error(-38020, "block number overflow while constructing sequence")
        })?;
        let number =
            overrides.number.map(|number| number.saturating_to()).unwrap_or(default_number);
        if number <= previous_number {
            return Err(simulate_error(
                -38020,
                format!("block numbers must be in order: {number} <= {previous_number}"),
            ));
        }

        let gap = number - previous_number - 1;
        let remaining = MAX_SIMULATE_BLOCKS as usize - sanitized.len();
        if gap as usize >= remaining {
            return Err(simulate_error(-38026, "too many blocks"));
        }
        for offset in 0..gap {
            let timestamp = previous_timestamp.checked_add(interval).ok_or_else(|| {
                simulate_error(-38021, "block timestamp overflow while filling number gap")
            })?;
            sanitized.push(SimBlock {
                block_overrides: Some(BlockOverrides {
                    number: Some(U256::from(default_number + offset)),
                    time: Some(timestamp),
                    ..Default::default()
                }),
                state_overrides: None,
                calls: Vec::new(),
            });
            previous_timestamp = timestamp;
        }

        let timestamp = match overrides.time {
            Some(timestamp) => timestamp,
            None => previous_timestamp.checked_add(interval).ok_or_else(|| {
                simulate_error(-38021, "block timestamp overflow while constructing sequence")
            })?,
        };
        if timestamp <= previous_timestamp {
            return Err(simulate_error(
                -38021,
                format!("block timestamps must be in order: {timestamp} <= {previous_timestamp}"),
            ));
        }

        overrides.number = Some(U256::from(number));
        overrides.time = Some(timestamp);
        block.block_overrides = Some(overrides);
        sanitized.push(block);
        previous_number = number;
        previous_timestamp = timestamp;
    }

    Ok(sanitized)
}

/// Whether an account override changes state, rather than only moving a precompile.
fn overrides_state(account: &AccountOverride) -> bool {
    account.balance.is_some()
        || account.nonce.is_some()
        || account.code.is_some()
        || account.state.is_some()
        || account.state_diff.as_ref().is_some_and(|diff| !diff.is_empty())
}

/// Validates and applies the `movePrecompileToAddress` overrides, as anvil does.
///
/// Returns the addresses EIP-2929 warms when precompiles moved: the protocol precompiles, not the
/// simulation-only destinations.
fn apply_precompile_moves(
    overrides: &StateOverride,
    precompiles: &mut PrecompilesMap,
) -> Result<Option<AddressSet>, EthApiError> {
    let mut moves = overrides
        .iter()
        .filter_map(|(source, account)| {
            account.move_precompile_to.map(|destination| (*source, destination))
        })
        .collect::<Vec<_>>();
    if moves.is_empty() {
        return Ok(None);
    }
    moves.sort_unstable();

    let addresses = precompiles.addresses().copied().collect::<AddressSet>();
    // Invalid sources take precedence over the more specific move errors.
    for (source, _) in &moves {
        if !addresses.contains(source) {
            return Err(simulate_error(-32000, format!("account {source} is not a precompile")));
        }
    }
    for (source, destination) in &moves {
        if source == destination {
            return Err(simulate_error(
                -38022,
                format!("cannot move precompile {source} to itself"),
            ));
        }
    }
    let mut destinations = Vec::with_capacity(moves.len());
    for (_, destination) in &moves {
        if destinations.contains(destination) {
            return Err(simulate_error(
                -38023,
                format!("multiple precompiles moved to {destination}"),
            ));
        }
        destinations.push(*destination);
    }

    precompiles.move_precompiles(moves.iter().copied()).map_err(
        |MovePrecompileError::NotAPrecompile(address)| {
            simulate_error(-32000, format!("account {address} is not a precompile"))
        },
    )?;
    // A dynamic lookup must not restore a precompile removed from its protocol address.
    let sources = Arc::new(moves.iter().map(|(source, _)| *source).collect::<AddressSet>());
    precompiles.map_precompile_lookup(move |address, previous| {
        if sources.contains(address) {
            None
        } else {
            previous.and_then(|lookup| lookup.lookup(address))
        }
    });

    Ok(Some(addresses))
}

/// The outcome of one simulated call.
struct SimulatedCall<Halt> {
    result: ExecutionResult<Halt>,
    /// The response logs with their index within the call.
    logs: Vec<(u64, PrimitiveLog)>,
    /// How many logs the call attempted, reverted frames included.
    attempted_logs: u64,
    /// Whether the call ran with the maximum nonce and no validation.
    max_nonce: bool,
}

/// Executes the calls of one simulated block, with anvil's gas rules, and builds the block.
#[expect(clippy::too_many_arguments, clippy::type_complexity)]
fn execute_calls<S, T, Net>(
    mut builder: S,
    state_provider: impl StateProvider,
    calls: Vec<RpcTxReq<T::Network>>,
    budget: &mut u64,
    chain_id: u64,
    validation: bool,
    max_blob_gas: u64,
    compute_state_root: bool,
    converter: &T,
) -> Result<
    (
        BlockBuilderOutcome<S::Primitives>,
        Vec<SimulatedCall<<<S::Executor as BlockExecutor>::Evm as Evm>::HaltReason>>,
    ),
    EthApiError,
>
where
    S: BlockBuilder<
        Executor: BlockExecutor<
            Evm: Evm<
                DB: Database<Error: Into<EthApiError>> + DatabaseCommit,
                Inspector = SimulationInspector,
            >,
        >,
    >,
    T: RpcConvert<Primitives = S::Primitives>,
    RpcTxReq<T::Network>: CallBatch<Net>,
{
    builder.apply_pre_execution_changes()?;

    let block_gas_limit = builder.evm().block().gas_limit();
    let is_amsterdam = builder.evm().cfg_env().enable_amsterdam_eip8037;
    let tx_gas_limit_cap = builder.evm().cfg_env().tx_gas_limit_cap.unwrap_or(u64::MAX);
    let mut results = Vec::with_capacity(calls.len());
    let mut cumulative_gas_used = 0u64;
    let mut regular_gas_used = 0u64;
    let mut state_gas_used = 0u64;
    let mut blob_gas_used = 0u64;

    for mut call in calls {
        let remaining_regular_gas = block_gas_limit.saturating_sub(regular_gas_used);
        let remaining_state_gas = block_gas_limit.saturating_sub(state_gas_used);
        let remaining_gas = if is_amsterdam {
            remaining_regular_gas.min(remaining_state_gas)
        } else {
            block_gas_limit.saturating_sub(cumulative_gas_used)
        };
        let requested_gas = call.as_ref().gas.unwrap_or(remaining_gas);
        let exceeds_gas_limit = if is_amsterdam {
            requested_gas.min(tx_gas_limit_cap) > remaining_regular_gas
                || requested_gas > remaining_state_gas
        } else {
            requested_gas > remaining_gas
        };
        if exceeds_gas_limit {
            return Err(EthApiError::other(EthSimulateError::BlockGasLimitExceeded));
        }
        // The request budget caps every call; the block gas limit is checked above. A fee payer
        // signs the gas limit, so a sponsored call keeps its own, or its sponsor would change.
        let execution_gas =
            if call.signs_gas() { requested_gas } else { requested_gas.min(*budget) };
        call.as_mut().gas = Some(execution_gas);

        let caller = call.as_ref().from.unwrap_or_default();
        let db = builder.evm_mut().db_mut();
        let state_nonce =
            db.basic(caller).map_err(Into::into)?.map(|account| account.nonce).unwrap_or_default();
        let max_nonce = !validation && call.as_ref().nonce.unwrap_or(state_nonce) == u64::MAX;
        // A call bumps the caller's nonce at the protocol level; a create bumps it in its frame,
        // which fails at the maximum nonce and leaves it in place.
        let wraps_nonce = max_nonce && call.as_ref().to.is_some_and(|to| to.is_call());
        // An omitted blob fee cap is zero. The Ethereum handler skips its validation and
        // charge when validation is disabled, while preserving BLOBBASEFEE during execution.
        let blob_count = call.as_ref().blob_versioned_hashes.as_ref().map_or(0, Vec::len) as u64;
        if blob_count > 0 {
            blob_gas_used = blob_gas_used.saturating_add(blob_count * DATA_GAS_PER_BLOB);
            if blob_gas_used > max_blob_gas {
                return Err(EthApiError::InvalidParams(format!(
                    "blob gas usage exceeds the limit of {max_blob_gas} gas per block."
                )));
            }
            call.as_mut().max_fee_per_blob_gas.get_or_insert(0);
        }
        let basefee = builder.evm().block().basefee();
        let tx = resolve_transaction::<_, _, Net>(
            call,
            execution_gas,
            basefee,
            chain_id,
            !validation,
            builder.evm_mut().db_mut(),
            converter,
        )?;
        // An empty envelope, so a layer-2 execution client charges no L1 cost.
        let tx = WithEncoded::new(Default::default(), tx);

        builder.evm_mut().inspector_mut().begin_transaction();
        let mut result = None;
        let gas_output = builder
            .execute_transaction_with_result_closure(tx, |executed| {
                result = Some(executed.result().result.clone());
            })
            .map_err(blob_fee_error)?;
        let result = result.expect("the committed transaction has a result");
        let (logs, attempted_logs) = builder
            .evm_mut()
            .inspector_mut()
            .finish_transaction(result.logs(), result.is_success());
        // A caller at the maximum nonce wraps to zero, as in anvil; revm leaves it in place.
        if wraps_nonce {
            let db = builder.evm_mut().db_mut();
            let mut info = db.basic(caller).map_err(Into::into)?.unwrap_or_default();
            info.nonce = 0;
            let mut account = Account::from(info);
            account.status = AccountStatus::Touched;
            db.commit(EvmState::from_iter([(caller, account)]));
        }

        let gas_used = gas_output.tx_gas_used();
        *budget = budget.saturating_sub(gas_used);
        cumulative_gas_used = cumulative_gas_used.saturating_add(gas_used);
        regular_gas_used = regular_gas_used.saturating_add(result.gas().block_regular_gas_used());
        state_gas_used = state_gas_used.saturating_add(gas_output.state_gas_used());
        results.push(SimulatedCall { result, logs, attempted_logs, max_nonce });
    }

    let outcome = if compute_state_root {
        builder.finish(state_provider, None)?
    } else {
        builder.finish(NoopProvider::default(), None)?
    };

    Ok((outcome, results))
}

/// Builds the response block: reth's block with anvil's call logs, halt messages, and nonces.
fn build_block<Err, T>(
    outcome: BlockBuilderOutcome<T::Primitives>,
    calls: Vec<SimulatedCall<HaltReasonFor<T::Evm>>>,
    return_full_transactions: bool,
    converter: &T,
) -> Result<SimulatedBlock<RpcBlock<T::Network>>, Err>
where
    Err: std::error::Error
        + FromEthApiError
        + FromEvmError<T::Evm>
        + From<T::Error>
        + Into<ErrorObject<'static>>,
    T: RpcConvert,
{
    let header = outcome.block.clone_sealed_header();
    let hashes =
        outcome.block.body().transactions().iter().map(|tx| *tx.tx_hash()).collect::<Vec<_>>();
    let results = calls.iter().map(|call| call.result.clone()).collect();
    let mut simulated = reth_simulate::build_simulated_block::<Err, _>(
        outcome.block,
        results,
        return_full_transactions.into(),
        converter,
    )?;

    let mut log_index = 0;
    let mut restored = Vec::new();
    for (index, (call, response)) in calls.into_iter().zip(&mut simulated.calls).enumerate() {
        response.logs = call
            .logs
            .into_iter()
            .map(|(offset, log)| Log {
                inner: log,
                block_hash: Some(header.hash()),
                block_number: Some(header.number()),
                block_timestamp: Some(header.timestamp()),
                transaction_hash: hashes.get(index).copied(),
                transaction_index: Some(index as u64),
                log_index: Some(log_index + offset),
                removed: false,
            })
            .collect();
        log_index += call.attempted_logs;
        if let ExecutionResult::Halt { reason, .. } = &call.result
            && let Some(error) = &mut response.error
        {
            error.message = halt_message(reason);
        }
        if call.max_nonce {
            restored.push((index, "nonce", u128::from(u64::MAX)));
        }
    }
    if return_full_transactions && !restored.is_empty() {
        restore_fields(&mut simulated.inner, &restored).map_err(Err::from_eth_err)?;
    }

    Ok(simulated)
}

/// The error message anvil reports for a halted call.
fn halt_message(reason: &impl fmt::Debug) -> String {
    let reason = format!("{reason:?}");
    if reason.starts_with("OutOfGas") {
        "out of gas".to_string()
    } else {
        format!("vm execution error: {reason}")
    }
}

/// Shows the values the calls asked for on the transactions that ran with others, as anvil does:
/// the maximum nonce, which revm cannot execute, so the transaction ran with nonce zero.
fn restore_fields<B: Serialize + DeserializeOwned>(
    block: &mut B,
    fields: &[(usize, &str, u128)],
) -> Result<(), EthApiError> {
    let mut value =
        serde_json::to_value(&*block).map_err(|error| EthApiError::EvmCustom(error.to_string()))?;
    if let Some(transactions) = value.get_mut("transactions").and_then(Value::as_array_mut) {
        for (index, field, value) in fields {
            if let Some(transaction) = transactions.get_mut(*index) {
                transaction[*field] = json!(format!("{value:#x}"));
            }
        }
    }
    *block =
        serde_json::from_value(value).map_err(|error| EthApiError::EvmCustom(error.to_string()))?;
    Ok(())
}

/// A log the response reports, canonical or a synthetic transfer log.
#[derive(Debug)]
struct SimulationLog {
    log: PrimitiveLog,
    index: u64,
    canonical: bool,
}

/// Collects the response logs of a simulated call: the canonical logs, in order with the
/// synthetic transfer logs `traceTransfers` adds, without journaling the latter.
///
/// It also warms anvil's precompile addresses when precompile moves changed the EVM's set, as
/// EIP-2929 warms the protocol precompiles, not the simulation-only destinations.
#[derive(Debug, Default)]
pub(crate) struct SimulationInspector {
    trace_transfers: bool,
    logs: Vec<SimulationLog>,
    checkpoints: Vec<usize>,
    next_index: u64,
    journal_log_count: usize,
    /// The addresses to warm at the start of every transaction, if precompiles moved.
    warm_precompiles: Option<AddressSet>,
    /// Whether the current transaction got its warm addresses.
    warmed: bool,
}

impl SimulationInspector {
    /// Creates the collector; `trace_transfers` adds the synthetic transfer logs.
    fn new(trace_transfers: bool) -> Self {
        Self { trace_transfers, ..Default::default() }
    }

    /// Resets the collector for the next transaction.
    fn begin_transaction(&mut self) {
        self.logs.clear();
        self.checkpoints.clear();
        self.next_index = 0;
        self.journal_log_count = 0;
        self.warmed = false;
    }

    /// Returns the response logs of the finished transaction and how many it attempted.
    fn finish_transaction(
        &mut self,
        canonical_logs: &[PrimitiveLog],
        success: bool,
    ) -> (Vec<(u64, PrimitiveLog)>, u64) {
        if success {
            self.append_remaining_canonical_logs(canonical_logs);
        } else {
            // A top-level revert discards the logs without a frame callback. Keep the attempted
            // count for the log indices of the calls after it.
            self.logs.clear();
        }
        let logs = std::mem::take(&mut self.logs);
        (logs.into_iter().map(|log| (log.index, log.log)).collect(), self.next_index)
    }

    fn push_log(&mut self, log: PrimitiveLog, canonical: bool) {
        self.logs.push(SimulationLog { log, index: self.next_index, canonical });
        self.next_index += 1;
    }

    fn push_canonical_log(&mut self, log: PrimitiveLog, journal_log_count: usize) {
        self.push_log(log, true);
        self.journal_log_count = journal_log_count;
    }

    /// Records the journal logs added since the last hook, such as precompile logs.
    fn sync_journal_logs(&mut self, logs: &[PrimitiveLog]) {
        self.journal_log_count = self.journal_log_count.min(logs.len());
        for log in &logs[self.journal_log_count..] {
            self.push_log(log.clone(), true);
        }
        self.journal_log_count = logs.len();
    }

    fn push_transfer(&mut self, from: Address, to: Address, value: U256) {
        if !self.trace_transfers || value.is_zero() {
            return;
        }
        let data = PrimitiveLog::new_unchecked(
            TRANSFER_LOG_EMITTER,
            vec![TRANSFER_EVENT_TOPIC, from.into_word(), to.into_word()],
            Bytes::from(value.to_be_bytes::<32>()),
        );
        self.push_log(data, false);
    }

    fn frame_start(&mut self) {
        self.checkpoints.push(self.logs.len());
    }

    fn frame_end(&mut self, success: bool, journal_log_count: usize) {
        let checkpoint = self.checkpoints.pop().expect("execution frame checkpoint exists");
        if !success {
            self.logs.truncate(checkpoint);
        }
        self.journal_log_count = journal_log_count;
    }

    fn append_remaining_canonical_logs(&mut self, canonical_logs: &[PrimitiveLog]) {
        let collected = self.logs.iter().filter(|log| log.canonical).count();
        for log in canonical_logs.iter().skip(collected) {
            self.push_log(log.clone(), true);
        }
    }
}

impl<CTX> Inspector<CTX> for SimulationInspector
where
    CTX: ContextTr,
{
    fn initialize_interp(&mut self, _interp: &mut Interpreter, context: &mut CTX) {
        if !self.warmed {
            self.warmed = true;
            if let Some(addresses) = &self.warm_precompiles {
                context.journal_mut().warm_precompiles(addresses);
            }
        }
        self.sync_journal_logs(context.journal().logs());
    }

    fn step(&mut self, _interp: &mut Interpreter, context: &mut CTX) {
        self.sync_journal_logs(context.journal().logs());
    }

    fn step_end(&mut self, _interp: &mut Interpreter, context: &mut CTX) {
        self.sync_journal_logs(context.journal().logs());
    }

    fn log(&mut self, context: &mut CTX, log: PrimitiveLog) {
        self.push_canonical_log(log, context.journal().logs().len());
    }

    fn call(&mut self, context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.sync_journal_logs(context.journal().logs());
        self.frame_start();
        if inputs.scheme.is_call()
            && let Some(value) = inputs.transfer_value()
        {
            self.push_transfer(inputs.transfer_from(), inputs.transfer_to(), value);
        }
        None
    }

    fn call_end(&mut self, context: &mut CTX, _inputs: &CallInputs, outcome: &mut CallOutcome) {
        self.sync_journal_logs(context.journal().logs());
        self.frame_end(outcome.instruction_result().is_ok(), context.journal().logs().len());
    }

    fn create(&mut self, context: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        self.sync_journal_logs(context.journal().logs());
        self.frame_start();
        if matches!(inputs.scheme(), CreateScheme::Create | CreateScheme::Create2 { .. })
            && let Ok(account) = context.journal_mut().load_account(inputs.caller())
        {
            let address = inputs.created_address(account.data.info.nonce);
            self.push_transfer(inputs.caller(), address, inputs.value());
        }
        None
    }

    fn create_end(
        &mut self,
        context: &mut CTX,
        _inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        self.sync_journal_logs(context.journal().logs());
        self.frame_end(
            outcome.instruction_result().is_ok() && outcome.address.is_some(),
            context.journal().logs().len(),
        );
    }

    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        self.push_transfer(contract, target, value);
    }
}

/// Fills the missing fields of a simulated call, as reth's `resolve_transaction` does, except
/// that a call batch without `to` stays a batch instead of becoming a contract creation.
fn resolve_transaction<DB, T, Net>(
    mut tx: RpcTxReq<T::Network>,
    default_gas_limit: u64,
    block_base_fee_per_gas: u64,
    chain_id: u64,
    disable_nonce_check: bool,
    db: &mut DB,
    converter: &T,
) -> Result<Recovered<<T::Primitives as NodePrimitives>::SignedTx>, EthApiError>
where
    DB: Database<Error: Into<EthApiError>>,
    T: RpcConvert,
    RpcTxReq<T::Network>: CallBatch<Net>,
{
    if tx.has_calls() {
        let from = tx.as_ref().from.unwrap_or_default();
        tx.as_mut().from = Some(from);
        if tx.as_ref().nonce.is_none() {
            let nonce = match tx.nonce_lane(from) {
                NonceLane::Account => {
                    db.basic(from).map_err(Into::into)?.map(|acc| acc.nonce).unwrap_or_default()
                }
                NonceLane::Expiring => 0,
                NonceLane::Storage(address, slot) => {
                    db.storage(address, slot).map_err(Into::into)?.saturating_to()
                }
            };
            tx.as_mut().nonce = Some(nonce);
        }
        if disable_nonce_check && tx.as_ref().nonce == Some(u64::MAX) {
            tx.as_mut().nonce = Some(0);
        }
        let request = tx.as_mut();
        request.gas.get_or_insert(default_gas_limit);
        request.chain_id.get_or_insert(chain_id);
        // Unspecified fees are zero, as the `eth_simulateV1` spec says.
        if request.gas_price.is_none() {
            request.max_fee_per_gas.get_or_insert(0);
            request.max_priority_fee_per_gas.get_or_insert(0);
        }
        let tx = converter
            .build_simulate_v1_transaction(tx)
            .map_err(|error| EthApiError::other(error.into()))?;
        return Ok(Recovered::new_unchecked(tx, from));
    }
    reth_simulate::resolve_transaction(
        tx,
        default_gas_limit,
        block_base_fee_per_gas,
        chain_id,
        disable_nonce_check,
        db,
        converter,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use revm::context::result::{HaltReason, OutOfGasError};

    fn block_at(number: Option<u64>, time: Option<u64>) -> SimBlock {
        SimBlock {
            block_overrides: Some(BlockOverrides {
                number: number.map(U256::from),
                time,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn numbers_and_times(blocks: &[SimBlock]) -> Vec<(u64, u64)> {
        blocks
            .iter()
            .map(|block| {
                let overrides = block.block_overrides.as_ref().unwrap();
                (overrides.number.unwrap().to::<u64>(), overrides.time.unwrap())
            })
            .collect()
    }

    #[test]
    fn sanitize_fills_gaps_with_the_interval() {
        let blocks = vec![block_at(Some(13), Some(1100)), SimBlock::default()];
        let sanitized = sanitize_blocks(blocks, 10, 1000, 12).unwrap();
        assert_eq!(numbers_and_times(&sanitized), [(11, 1012), (12, 1024), (13, 1100), (14, 1112)]);
    }

    fn code(error: EthApiError) -> i32 {
        ErrorObject::from(error).code()
    }

    #[test]
    fn sanitize_rejects_out_of_order_blocks() {
        let error = sanitize_blocks(vec![block_at(Some(10), None)], 10, 1000, 12).unwrap_err();
        assert_eq!(code(error), -38020);
        let error = sanitize_blocks(vec![block_at(None, Some(1000))], 10, 1000, 12).unwrap_err();
        assert_eq!(code(error), -38021);
        let error = sanitize_blocks(vec![block_at(Some(267), None)], 10, 1000, 12).unwrap_err();
        assert_eq!(code(error), -38026);
    }

    #[test]
    fn halt_messages_follow_anvil() {
        assert_eq!(halt_message(&HaltReason::OutOfGas(OutOfGasError::Basic)), "out of gas");
        assert_eq!(
            halt_message(&HaltReason::InvalidFEOpcode),
            "vm execution error: InvalidFEOpcode"
        );
    }
}
