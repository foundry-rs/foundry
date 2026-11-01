//! In-memory blockchain backend.
use self::{in_memory_db::StateRootDb, state::trie_storage, tempo::TempoChain};
use crate::{
    ForkChoice, NodeConfig, PrecompileFactory,
    config::{ForkTransactionReplay, PruneStateHistoryConfig, source_hardfork},
    eth::{
        backend::{
            cheats::{CheatsManager, NextBlockOverrides},
            db::{
                AccountState, BLOCKHASH_HISTORY, BlockchainDb, CacheDB, DatabaseRef, Db, DbAccount,
                EmptyDb, EvmDb, MaybeFullDatabase, RevertStateSnapshotAction, SerializableState,
                StateDb, apply_block_overrides, apply_state_overrides,
            },
            env::{
                BlobExcessGasAndPrice, BlockEnv, CfgEnv, EvmEnv, SpecId,
                apply_chain_specific_tx_replay_env_changes_for_chain, block_env_from_header,
            },
            evm::{
                AnvilEvmNetwork, AnyInspector, EthEvmNetwork, EvmFamily, EvmOptions, Executed,
                PreparedTx, TempoEvmNetwork, TempoSimulation, TxContext, TxPayload, bsc_p256,
                build_evm, with_evm_network,
            },
            executor::{
                AnvilBlockExecutor, BlockExecutionKind, BlockExecutionResult,
                EthereumBlockTransitions, ExecutedPoolTransactions, PoolTxGasConfig, build_receipt,
            },
            fork::{ClientFork, ForkEndpointIdentity},
            genesis::GenesisConfig,
            mem::{
                state::{
                    StateRootCache, state_root, state_trie_witness, storage_root, trie_accounts,
                },
                storage::MinedTransactionReceipt,
            },
            notifications::{ChainNotification, ChainNotifications, NewBlockNotification},
            replay::{
                ExecutedHistoricalReplay, HistoricalReplayTransaction,
                PreparedForkTransactionReplay, execute_historical_replay,
                prepare_fork_transaction_replay,
            },
            time::{PendingBlockTimestamp, TimeManager, TimeSnapshot, utc_from_secs},
            validate::TransactionValidator,
        },
        error::{BlockchainError, ErrDetail, InvalidTransactionError},
        fees::{FeeDetails, FeeManager, FeeSnapshot, MIN_SUGGESTED_PRIORITY_FEE},
        macros::node_info,
        pool::transactions::PoolTransaction,
        preserve_simulation_request_fields,
    },
    mem::{
        inspector::{AnvilInspector, InspectorTxConfig},
        storage::{BlockchainStorage, InMemoryBlockStates, MinedBlockOutcome},
    },
};
use alloy_chains::NamedChain;
use alloy_consensus::{
    Blob, BlockBody, BlockHeader, EnvKzgSettings, Header, Signed, Transaction as TransactionTrait,
    TransactionEnvelope, TrieAccount, TxEip4844Variant, TxEnvelope, TxReceipt, Typed2718,
    constants::EMPTY_WITHDRAWALS,
    proofs::{calculate_receipt_root, calculate_transaction_root},
    transaction::Recovered,
};
use alloy_eips::{
    BlockNumHash, eip2935, eip4788,
    eip4844::{DATA_GAS_PER_BLOB, kzg_to_versioned_hash},
    eip6110::MAINNET_DEPOSIT_CONTRACT_ADDRESS,
    eip7002, eip7251,
    eip7685::EMPTY_REQUESTS_HASH,
    eip7840::BlobParams,
    eip7910::SystemContract,
    eip7928::{BlockAccessList, EMPTY_BLOCK_ACCESS_LIST_HASH, compute_block_access_list_hash},
};
use alloy_genesis::Genesis;
use alloy_network::{
    AnyHeader, AnyRpcBlock, AnyRpcHeader, AnyRpcTransaction, AnyTxEnvelope, AnyTxType,
    BlockResponse, Network, NetworkTransactionBuilder, ReceiptResponse, UnknownTxEnvelope,
    UnknownTypedTransaction,
};
use alloy_primitives::{
    Address, B256, Bloom, Bytes, KECCAK256_EMPTY, Signature, TxKind, U256, address, hex, keccak256,
    map::{AddressMap, B256Set, HashMap, HashSet},
};
use alloy_rlp::{Decodable, Encodable};
use alloy_rpc_types::{
    AccessList, Block as AlloyBlock, BlockId, BlockNumberOrTag as BlockNumber, BlockOverrides,
    BlockTransactions, EIP1186AccountProofResponse as AccountProof,
    EIP1186StorageProof as StorageProof, Filter, Header as AlloyHeader, Index, Log, Transaction,
    TransactionReceipt,
    anvil::Forking,
    debug::ExecutionWitness,
    request::TransactionRequest,
    serde_helpers::JsonStorageKey,
    simulate::{
        MAX_SIMULATE_BLOCKS, SimBlock, SimCallResult, SimulateError, SimulatePayload,
        SimulatedBlock,
    },
    state::{EvmOverrides, StateOverride},
    trace::{
        filter::TraceFilter,
        geth::{
            CallConfig, FourByteFrame, GethDebugBuiltInTracerType, GethDebugTracerConfig,
            GethDebugTracerType, GethDebugTracingCallOptions, GethDebugTracingOptions, GethTrace,
            NoopFrame, TraceResult,
        },
        opcode::{BlockOpcodeGas, TransactionOpcodeGas},
        parity::{
            LocalizedTransactionTrace, TraceResults, TraceResultsWithTransactionHash, TraceType,
        },
    },
};
use alloy_rpc_types_eth::{AccountInfo as RpcAccountInfo, Bundle, EthCallResponse};
use alloy_rpc_types_mev::{EthCallBundle, EthCallBundleResponse, EthCallBundleTransactionResult};
use alloy_serde::{OtherFields, WithOtherFields};
use alloy_sol_types::SolCall;
use alloy_trie::{HashBuilder, Nibbles, proof::ProofRetainer, root::state_root_ref_unhashed};
use anvil_core::eth::{
    block::{Block, BlockInfo, canonical_block, create_block},
    transaction::{MaybeImpersonatedTransaction, PendingTransaction, TransactionInfo},
};
use anvil_rpc::error::{ErrorCode, RpcError};
use chrono::Datelike;
use evm2::{
    AccountInfo,
    evm::{BalContext, BlockAccessIndex, BlockStateAccumulator, PendingState, StateChangeSource},
    interpreter::InstrStop,
};
use evm2_inspectors::{
    access_list::AccessListInspector,
    opcode::OpcodeGasInspector,
    tracing::{FourByteInspector, GethTraceBuilder, TracingInspector, TracingInspectorConfig},
};
use eyre::{Context, Result};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use foundry_evm_core::{
    backend::{DatabaseError, DatabaseResult},
    constants::{DEFAULT_CREATE2_DEPLOYER, DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE},
    decode::RevertDecoder,
    utils::{get_blob_base_fee_update_fraction_by_spec_id, get_blob_params_by_hardfork},
};
use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork};
use foundry_evm_networks::{NetworkConfigs, arbitrum};
use foundry_evm_traces::CallTraceDecoder;
use foundry_primitives::{
    FoundryHeader, FoundryNetwork, FoundryReceiptEnvelope, FoundryTransactionRequest,
    FoundryTxEnvelope, FoundryTxReceipt, FoundryTypedTx, TempoTransactionRequest,
};
use futures::channel::mpsc::{UnboundedSender, unbounded};
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use std::{
    collections::BTreeMap,
    fmt::{self, Debug},
    io::{Read, Write},
    ops::Mul,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use storage::{Blockchain, DEFAULT_HISTORY_LIMIT, MinedTransaction};
use tempo_hardfork::TempoHardfork;
use tempo_precompiles::{
    NONCE_PRECOMPILE_ADDRESS, TIP_FEE_MANAGER_ADDRESS,
    nonce::NonceManager,
    storage::Handler,
    tip_fee_manager::{IFeeManager, TipFeeManager},
    tip20::{ITIP20, TIP20Token},
    tip20_factory::TIP20Factory,
};
use tempo_primitives::{
    AASigned, SignatureType, TEMPO_TX_TYPE_ID, TempoSignature,
    transaction::{
        KeychainSignature, PrimitiveSignature,
        tt_signature::{P256SignatureWithPreHash, WebAuthnSignature},
    },
};
#[cfg(test)]
use tokio::sync::Notify;
use tokio::{sync::RwLock as AsyncRwLock, task::JoinSet};

#[cfg(feature = "base")]
use evm2::bytecode::Bytecode;

#[cfg(feature = "js-tracer")]
use crate::eth::backend::evm::{TxFor, TypesFor, block_env};
#[cfg(feature = "js-tracer")]
use evm2_inspectors::tracing::{TransactionContext, js::JsInspector};

#[cfg(any(feature = "base", feature = "optimism"))]
use foundry_primitives::get_deposit_tx_parts;
#[cfg(any(feature = "base", feature = "optimism"))]
use op_alloy_consensus::DEPOSIT_TX_TYPE_ID;

#[cfg(feature = "base")]
use crate::eth::backend::evm::BaseEvmNetwork;
#[cfg(feature = "base")]
use alloy_eips::Encodable2718;
#[cfg(feature = "base")]
use base_common_chains::ChainConfig;
#[cfg(feature = "base")]
use base_common_consensus::{
    BaseTransactionInfo, BaseTxEnvelope, EIP8130_REJECTION_MSG, Eip8130Constants, Eip8130Signed,
    Predeploys,
};
#[cfg(feature = "base")]
use base_common_eip8130::{IntrinsicGas, IntrinsicGasInput, NonceManagerSlots};
#[cfg(feature = "base")]
use base_common_rpc_types::{
    EIP8130_PRE_ZENITH_RPC_ERROR, Eip8130Nonce, Eip8130ReceiptFields,
    Transaction as BaseRpcTransaction,
};
#[cfg(feature = "base")]
use base_evm2::L1BlockInfo;
#[cfg(feature = "base")]
use foundry_evm_hardforks::BaseUpgrade;

#[cfg(feature = "optimism")]
use foundry_evm_hardforks::OpHardfork;
#[cfg(feature = "optimism")]
use foundry_evm_networks::NetworkVariant;
#[cfg(feature = "optimism")]
use op_alloy_consensus::POST_EXEC_TX_TYPE_ID;

/// Fully prepared fork replacement awaiting an atomic backend commit.
pub(crate) struct StagedForkReset {
    node_config: NodeConfig,
    db: Box<dyn Db>,
    fees: FeeManager,
    evm_env: EvmEnv,
    fork: ClientFork,
    timestamp: u64,
    discard_old_cached_state: bool,
    invalidated_cache_namespaces: Vec<ForkCacheNamespace>,
    flush_old_cache: bool,
    cache_lease: StagedForkCacheLease,
}

/// Fully prepared in-memory replacement awaiting an atomic backend commit.
pub(crate) struct StagedMemoryReset<N: Network> {
    node_config: NodeConfig,
    db: Box<dyn Db>,
    fees: FeeManager,
    evm_env: EvmEnv,
    hardfork: FoundryHardfork,
    storage: BlockchainStorage<N>,
    timestamp: u64,
    flush_old_cache: bool,
}

/// Identifies the endpoint that supplied the most recently committed fork.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ForkCacheSource {
    rpc_url: String,
    endpoint_identity: ForkEndpointIdentity,
}

impl ForkCacheSource {
    fn from_fork(fork: &ClientFork) -> Option<Self> {
        let config = fork.config.read();
        Some(Self {
            rpc_url: config.eth_rpc_url()?.to_string(),
            endpoint_identity: config.endpoint_identity,
        })
    }

    fn authoritative_identity_changed_at_same_url(
        &self,
        rpc_url: &str,
        endpoint_identity: ForkEndpointIdentity,
    ) -> bool {
        // `hardfork` is populated only when `anvil_nodeInfo` succeeds, which makes the complete
        // endpoint identity authoritative. Anonymous RPC endpoints intentionally retain Foundry's
        // existing cache behavior when reused through the same URL.
        let authoritative =
            self.endpoint_identity.is_authoritative() || endpoint_identity.is_authoritative();
        self.rpc_url == rpc_url && authoritative && self.endpoint_identity != endpoint_identity
    }
}

/// Identifies one endpoint's persisted cache files across all blocks of a source chain.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ForkCacheNamespace {
    chain_cache_dir: PathBuf,
    file_name: String,
}

impl ForkCacheNamespace {
    fn new(source_chain_id: u64, rpc_url: &str) -> Option<Self> {
        Some(Self {
            chain_cache_dir: foundry_config::Config::foundry_chain_cache_dir(source_chain_id)?,
            file_name: format!("storage-{:x}.json", keccak256(rpc_url)),
        })
    }

    fn invalidate(&self) -> Result<(), BlockchainError> {
        let entries = match std::fs::read_dir(&self.chain_cache_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(BlockchainError::Internal(format!(
                    "failed to inspect fork cache at {}: {err}",
                    self.chain_cache_dir.display()
                )));
            }
        };

        for entry in entries {
            let entry = entry.map_err(|err| {
                BlockchainError::Internal(format!(
                    "failed to inspect fork cache at {}: {err}",
                    self.chain_cache_dir.display()
                ))
            })?;
            let file_type = entry.file_type().map_err(|err| {
                BlockchainError::Internal(format!(
                    "failed to inspect fork cache entry at {}: {err}",
                    entry.path().display()
                ))
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let cache_path = entry.path().join(&self.file_name);
            if let Err(err) = std::fs::remove_file(&cache_path)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                return Err(BlockchainError::Internal(format!(
                    "failed to invalidate fork cache at {}: {err}",
                    cache_path.display()
                )));
            }
        }
        Ok(())
    }
}

/// Keeps a staged fork from persisting remote state unless it is committed.
#[derive(Clone, Debug, Default)]
struct StagedForkCacheLease(Option<Arc<StagedForkCacheLeaseInner>>);

#[derive(Debug)]
struct StagedForkCacheLeaseInner {
    db: BlockchainDb,
    cache_path: PathBuf,
    armed: AtomicBool,
}

impl StagedForkCacheLease {
    fn new(db: BlockchainDb, cache_path: Option<PathBuf>) -> Self {
        Self(cache_path.map(|cache_path| {
            Arc::new(StagedForkCacheLeaseInner { db, cache_path, armed: AtomicBool::new(true) })
        }))
    }

    fn for_db(db: &BlockchainDb) -> Self {
        Self::new(db.clone(), db.cache().cache_path().map(Path::to_path_buf))
    }

    fn disarm(&self) {
        if let Some(inner) = &self.0 {
            inner.armed.store(false, Ordering::Release);
        }
    }

    fn rollback(&self) -> Result<(), BlockchainError> {
        let Some(inner) = &self.0 else { return Ok(()) };
        // A clone owned by an in-flight database user must outlive that user's SharedBackend.
        // Leave cleanup armed for the final owner instead of racing its eventual cache flush.
        if Arc::strong_count(inner) == 1 && inner.armed.load(Ordering::Acquire) {
            inner.invalidate()?;
            inner.armed.store(false, Ordering::Release);
        }
        Ok(())
    }
}

impl StagedForkCacheLeaseInner {
    fn invalidate(&self) -> Result<(), BlockchainError> {
        self.db.db().clear();
        self.db.cache().flush();
        // Forge can store the block cache as a file, so a child endpoint cache cannot exist.
        if let Err(err) = std::fs::remove_file(&self.cache_path)
            && !matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            )
        {
            return Err(BlockchainError::Internal(format!(
                "failed to invalidate fork cache at {}: {err}",
                self.cache_path.display()
            )));
        }
        Ok(())
    }
}

impl Drop for StagedForkCacheLeaseInner {
    fn drop(&mut self) {
        if self.armed.swap(false, Ordering::AcqRel)
            && let Err(err) = self.invalidate()
        {
            warn!(target: "backend", %err, "failed to roll back staged fork cache");
        }
    }
}

/// Couples an asynchronous staged-database user to its rollback lease.
///
/// Fields drop in declaration order, so the database handle (and its SharedBackend) is released
/// before the lease can perform final cache cleanup.
struct StagedForkDbUser<D> {
    db: Option<Arc<AsyncRwLock<D>>>,
    cache_lease: StagedForkCacheLease,
}

impl<D> Clone for StagedForkDbUser<D> {
    fn clone(&self) -> Self {
        Self { db: self.db.clone(), cache_lease: self.cache_lease.clone() }
    }
}

impl<D> StagedForkDbUser<D> {
    const fn db(&self) -> &Arc<AsyncRwLock<D>> {
        self.db.as_ref().expect("staged fork database must be present until drop")
    }
}

impl<D> Drop for StagedForkDbUser<D> {
    fn drop(&mut self) {
        // Explicitly release the SharedBackend before `cache_lease` can invalidate its cache.
        drop(self.db.take());
    }
}

#[cfg(feature = "monad")]
pub(crate) type MonadReplayContext = monad_evm2::MonadChainContext;
// Opaque stand-in that keeps feature-independent replay context plumbing type-stable.
#[cfg(not(feature = "monad"))]
#[derive(Clone)]
pub(crate) struct MonadReplayContext;

#[cfg(feature = "monad")]
enum MonadExecutionContext<'a> {
    Exact(Box<MonadReplayContext>),
    Next(&'a mut MonadReplayContext),
}

#[cfg(not(feature = "monad"))]
struct MonadExecutionContext<'a> {
    _marker: std::marker::PhantomData<&'a mut MonadReplayContext>,
}

#[cfg_attr(not(feature = "monad"), allow(dead_code))]
struct EnvelopeExecution<'a> {
    monad_context: Option<MonadExecutionContext<'a>>,
    hardfork: FoundryHardfork,
    /// Whether the envelope is trusted mined history, whose protocol transactions replay.
    protocol_replay: bool,
}

impl<'a> EnvelopeExecution<'a> {
    const fn transaction(
        monad_context: Option<MonadExecutionContext<'a>>,
        hardfork: FoundryHardfork,
    ) -> Self {
        Self { monad_context, hardfork, protocol_replay: false }
    }

    const fn replay(
        monad_context: Option<MonadExecutionContext<'a>>,
        hardfork: FoundryHardfork,
    ) -> Self {
        Self { monad_context, hardfork, protocol_replay: true }
    }
}

#[cfg(feature = "monad")]
fn monad_execution_context_at(
    context: Option<&MonadReplayContext>,
    current_tx_index: usize,
) -> Option<MonadExecutionContext<'static>> {
    context.map(|context| {
        let mut context = context.clone();
        context.current_tx_index = current_tx_index;
        MonadExecutionContext::Exact(Box::new(context))
    })
}

#[cfg(not(feature = "monad"))]
const fn monad_execution_context_at(
    _context: Option<&MonadReplayContext>,
    _current_tx_index: usize,
) -> Option<MonadExecutionContext<'static>> {
    None
}

#[cfg(feature = "monad")]
const fn next_monad_context(context: &mut MonadReplayContext) -> MonadExecutionContext<'_> {
    MonadExecutionContext::Next(context)
}

#[cfg(not(feature = "monad"))]
const fn next_monad_context(_context: &mut MonadReplayContext) -> MonadExecutionContext<'_> {
    MonadExecutionContext { _marker: std::marker::PhantomData }
}

/// Maximum cumulative gas available to one `eth_simulateV1` request.
const SIMULATE_GAS_CAP: u64 = 50_000_000;
const SEPOLIA_DEPOSIT_CONTRACT_ADDRESS: Address =
    address!("7f02c3e3c98b133055b8b348b2ac625669ed295d");
const HOLESKY_DEPOSIT_CONTRACT_ADDRESS: Address = Address::repeat_byte(0x42);

/// Fixed transaction context for direct Tempo RPC simulations.
const TEMPO_RPC_SIMULATION_CONTEXT: B256 = B256::new(*b"TEMPO_RPC_SIMULATION_MPP_CONTEXT");

/// Fee and limit fields of a call request, resolved against the call environment.
#[derive(Clone, Copy, Debug)]
struct CallFees {
    gas_price: u128,
    max_priority_fee_per_gas: u128,
    max_fee_per_blob_gas: u128,
    gas_limit: u64,
    chain_id: u64,
}

/// Result of a call executed against a state without committing it.
#[derive(Clone, Debug)]
pub struct CallOutcome {
    /// How execution stopped.
    pub exit: InstrStop,
    /// Return or revert data.
    pub output: Bytes,
    /// Gas used by the transaction.
    pub gas_used: u128,
    /// State changes of the call.
    pub state: PendingState,
}

impl From<Executed> for CallOutcome {
    fn from(Executed { result, state }: Executed) -> Self {
        Self {
            exit: result.stop,
            gas_used: result.tx_gas_used() as u128,
            output: result.output,
            state,
        }
    }
}

/// Outputs of a block executed by [`Backend::execute_with_block_executor`].
struct ExecutedBlock {
    pool: ExecutedPoolTransactions<FoundryTxEnvelope>,
    result: BlockExecutionResult,
    state: BlockStateAccumulator,
    bal: Option<BlockAccessList>,
}

/// Deposit fields of an OP-stack call request.
#[cfg(any(feature = "base", feature = "optimism"))]
type DepositParts = foundry_primitives::DepositTransactionParts;
#[cfg(not(any(feature = "base", feature = "optimism")))]
type DepositParts = std::convert::Infallible;

/// EIP-2780 base cost of a transaction.
const EIP2780_TX_BASE_COST: u64 = 12_000;

/// Returns whether `tx` is an OP-stack deposit.
const fn is_deposit(tx: &FoundryTxEnvelope) -> bool {
    #[cfg(any(feature = "base", feature = "optimism"))]
    if let FoundryTxEnvelope::Deposit(_) = tx {
        return true;
    }
    let _ = tx;
    false
}

/// Applies `f` to the fields of a simulated transaction, keeping a Tempo mock signature and the
/// representative authentication blobs of a Base EIP-8130 simulation.
fn map_simulated_tx(
    tx: FoundryTxEnvelope,
    f: impl FnOnce(&mut FoundryTypedTx) -> Result<(), BlockchainError>,
) -> Result<FoundryTxEnvelope, BlockchainError> {
    #[cfg(feature = "base")]
    if let FoundryTxEnvelope::Eip8130(signed) = tx {
        let sender_auth = signed.sender_auth().clone();
        let payer_auth = signed.payer_auth().clone();
        let mut typed = FoundryTypedTx::Eip8130(signed.into_tx());
        f(&mut typed)?;
        let FoundryTypedTx::Eip8130(tx) = typed else { unreachable!("EIP-8130 stays EIP-8130") };
        return Ok(FoundryTxEnvelope::Eip8130(Eip8130Signed::new(tx, sender_auth, payer_auth)));
    }
    if let FoundryTxEnvelope::Tempo(signed) = tx {
        let (tx, signature, _) = signed.into_parts();
        let mut typed = FoundryTypedTx::Tempo(tx);
        f(&mut typed)?;
        let FoundryTypedTx::Tempo(tx) = typed else { unreachable!("Tempo stays Tempo") };
        return Ok(FoundryTxEnvelope::Tempo(tx.into_signed(signature)));
    }
    let mut typed = FoundryTypedTx::from(tx);
    f(&mut typed)?;
    Ok(typed.into_impersonated())
}

/// Returns a simulated transaction with `gas_limit`.
fn with_gas_limit(tx: FoundryTxEnvelope, gas_limit: u64) -> FoundryTxEnvelope {
    let set = |typed: &mut FoundryTypedTx| {
        match typed {
            FoundryTypedTx::Legacy(tx) => tx.gas_limit = gas_limit,
            FoundryTypedTx::Eip2930(tx) => tx.gas_limit = gas_limit,
            FoundryTypedTx::Eip1559(tx) => tx.gas_limit = gas_limit,
            FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844(tx)) => tx.gas_limit = gas_limit,
            FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844WithSidecar(tx)) => {
                tx.tx.gas_limit = gas_limit
            }
            FoundryTypedTx::Eip7702(tx) => tx.gas_limit = gas_limit,
            FoundryTypedTx::Celo(tx) => tx.inner.gas_limit = gas_limit,
            FoundryTypedTx::Tempo(tx) => tx.gas_limit = gas_limit,
            #[cfg(any(feature = "base", feature = "optimism"))]
            FoundryTypedTx::Deposit(tx) => tx.gas_limit = gas_limit,
            #[cfg(feature = "base")]
            FoundryTypedTx::Eip8130(tx) => tx.gas_limit = gas_limit,
            #[allow(unreachable_patterns)]
            _ => {}
        }
        Ok(())
    };
    map_simulated_tx(tx, set).expect("setting the gas limit is infallible")
}

/// Returns a simulated transaction with `access_list`, upgrading a legacy transaction to EIP-2930.
fn with_access_list(
    tx: FoundryTxEnvelope,
    access_list: AccessList,
) -> Result<FoundryTxEnvelope, BlockchainError> {
    map_simulated_tx(tx, |typed| {
        match typed {
            FoundryTypedTx::Legacy(tx) => {
                *typed = FoundryTypedTx::Eip2930(alloy_consensus::TxEip2930 {
                    chain_id: tx.chain_id.unwrap_or_default(),
                    nonce: tx.nonce,
                    gas_price: tx.gas_price,
                    gas_limit: tx.gas_limit,
                    to: tx.to,
                    value: tx.value,
                    access_list,
                    input: std::mem::take(&mut tx.input),
                })
            }
            FoundryTypedTx::Eip2930(tx) => tx.access_list = access_list,
            FoundryTypedTx::Eip1559(tx) => tx.access_list = access_list,
            FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844(tx)) => {
                tx.access_list = access_list
            }
            FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844WithSidecar(tx)) => {
                tx.tx.access_list = access_list
            }
            FoundryTypedTx::Eip7702(tx) => tx.access_list = access_list,
            FoundryTypedTx::Celo(tx) => tx.inner.access_list = access_list,
            FoundryTypedTx::Tempo(tx) => tx.access_list = access_list,
            #[allow(unreachable_patterns)]
            _ => {}
        }
        Ok(())
    })
}

/// Returns a simulated transaction with `nonce`.
fn with_nonce(tx: FoundryTxEnvelope, nonce: u64) -> FoundryTxEnvelope {
    let set = |typed: &mut FoundryTypedTx| {
        match typed {
            FoundryTypedTx::Legacy(tx) => tx.nonce = nonce,
            FoundryTypedTx::Eip2930(tx) => tx.nonce = nonce,
            FoundryTypedTx::Eip1559(tx) => tx.nonce = nonce,
            FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844(tx)) => tx.nonce = nonce,
            FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844WithSidecar(tx)) => {
                tx.tx.nonce = nonce
            }
            FoundryTypedTx::Eip7702(tx) => tx.nonce = nonce,
            FoundryTypedTx::Celo(tx) => tx.inner.nonce = nonce,
            FoundryTypedTx::Tempo(tx) => tx.nonce = nonce,
            #[allow(unreachable_patterns)]
            _ => {}
        }
        Ok(())
    };
    map_simulated_tx(tx, set).expect("setting the nonce is infallible")
}

/// Returns whether executing `tx` increments the sender's protocol nonce before its call.
fn uses_protocol_call_nonce(tx: &FoundryTxEnvelope) -> bool {
    match tx {
        FoundryTxEnvelope::Tempo(tx) => {
            let tx = tx.tx();
            tx.nonce_key.is_zero() && tx.calls.first().is_some_and(|call| call.to.is_call())
        }
        tx if is_deposit(tx) => false,
        // EIP-8130 transactions count nonces in the nonce manager or the protocol lane before
        // any phase runs, and their calls do not start from the sender's protocol nonce.
        #[cfg(feature = "base")]
        FoundryTxEnvelope::Eip8130(_) => false,
        tx => tx.kind().is_call(),
    }
}

/// Describes an exceptional halt with REVM's `HaltReason` messages.
pub(crate) fn halt_reason(stop: InstrStop) -> String {
    match stop {
        InstrStop::OutOfGas => "out of gas",
        InstrStop::MemoryOOG => "out of gas: memory expansion",
        InstrStop::MemoryLimitOOG => "out of gas: memory limit exceeded",
        InstrStop::PrecompileOOG => "out of gas: precompile",
        InstrStop::InvalidOperandOOG => "out of gas: invalid operand",
        InstrStop::ReentrancySentryOOG => "out of gas: reentrancy sentry",
        InstrStop::OpcodeNotFound | InstrStop::InvalidImmediateEncoding => "opcode not found",
        InstrStop::InvalidFEOpcode => "invalid 0xFE opcode",
        InstrStop::InvalidJump => "invalid jump destination",
        InstrStop::NotActivated => "feature or opcode not activated",
        InstrStop::StackUnderflow => "stack underflow",
        InstrStop::StackOverflow => "stack overflow",
        InstrStop::OutOfOffset => "out of offset",
        InstrStop::CreateCollision => "create collision",
        InstrStop::PrecompileError => "precompile error",
        InstrStop::NonceOverflow => "nonce overflow",
        InstrStop::CreateContractSizeLimit => "create contract size limit",
        InstrStop::CreateContractStartingWithEF => "create contract starting with 0xEF",
        InstrStop::CreateInitCodeSizeLimit => "create initcode size limit",
        InstrStop::StateChangeDuringStaticCall => "state change during static call",
        InstrStop::CallNotAllowedInsideStatic => "call not allowed inside static call",
        InstrStop::OutOfFunds => "out of funds",
        InstrStop::CallTooDeep => "call too deep",
        InstrStop::FatalExternalError => "fatal external error",
        stop => return format!("{stop:?}"),
    }
    .to_string()
}

/// Applies a transaction's detached state changes to `db`.
fn commit_pending<DB: DatabaseRef>(db: &mut CacheDB<DB>, state: &PendingState) {
    let mut changes = BlockStateAccumulator::new();
    let Ok(()) = state.visit(&mut changes);
    db.commit(&changes);
}

struct PreparedCall {
    evm_env: EvmEnv,
    tx: PreparedTx,
    simulated_tempo_tx: Option<AASigned>,
    #[cfg(feature = "base")]
    simulated_envelope: Option<FoundryTxEnvelope>,
}

/// Gives a simulated Tempo request the identity of its response envelope.
fn apply_tempo_envelope_identity(tx: &mut PreparedTx, simulated_tx: Option<&AASigned>) {
    if let (Some(tempo), Some(simulated_tx)) = (tx.tempo.as_mut(), simulated_tx) {
        tempo.unique_tx_identifier = simulated_tx.expiring_nonce_hash(tx.sender);
    }
}

#[derive(Default)]
struct TypedCallOverrides {
    gas_limit: Option<u64>,
    access_list: Option<AccessList>,
    disable_fee_charge: bool,
}

pub(crate) struct GasEstimateCallOptions {
    gas_limit: u64,
    disable_fee_charge: bool,
    monad_context: Option<MonadReplayContext>,
}

impl GasEstimateCallOptions {
    pub(crate) const fn new(
        gas_limit: u64,
        disable_fee_charge: bool,
        monad_context: Option<MonadReplayContext>,
    ) -> Self {
        Self { gas_limit, disable_fee_charge, monad_context }
    }
}

#[cfg(feature = "base")]
mod base;
pub mod cache;
pub mod fork_db;
pub mod in_memory_db;
pub mod inspector;
#[cfg(feature = "monad")]
mod monad;
pub mod state;
pub mod storage;
mod tempo;

#[cfg(feature = "base")]
pub(crate) use base::eip8130_max_fee_charge;

// Gas per transaction not creating a contract.
pub const MIN_TRANSACTION_GAS: u128 = 21000;
// Gas per transaction creating a contract.
pub const MIN_CREATE_GAS: u128 = 53000;
/// Standalone Base L1 base fee, in wei.
#[cfg(feature = "base")]
const DEFAULT_BASE_L1_BASE_FEE: u64 = 1_000_000_000;
/// Standalone Base L1 base-fee scalar, with six decimal places.
#[cfg(feature = "base")]
const DEFAULT_BASE_L1_FEE_SCALAR: u32 = 1_000_000;

fn tempo_nonce(
    state: &dyn DatabaseRef,
    caller: Address,
    nonce_key: U256,
) -> Result<u64, BlockchainError> {
    if nonce_key.is_zero() {
        return Ok(state.basic_ref(caller)?.map(|account| account.nonce).unwrap_or_default());
    }
    if nonce_key == U256::MAX {
        return Ok(0);
    }
    let slot = NonceManager::new().nonces[caller][nonce_key].slot();
    Ok(state.storage_ref(NONCE_PRECOMPILE_ADDRESS, slot)?.saturating_to())
}

fn mock_tempo_signature(
    key_type: SignatureType,
    key_data: Option<Bytes>,
    key_id: Option<Address>,
    caller: Address,
    is_t1c: bool,
) -> TempoSignature {
    let signature = match key_type {
        SignatureType::Secp256k1 => {
            PrimitiveSignature::Secp256k1(Signature::new(U256::ZERO, U256::ZERO, false))
        }
        SignatureType::P256 => PrimitiveSignature::P256(P256SignatureWithPreHash {
            r: B256::ZERO,
            s: B256::ZERO,
            pub_key_x: B256::ZERO,
            pub_key_y: B256::ZERO,
            pre_hash: false,
        }),
        SignatureType::WebAuthn => {
            const CLIENT_JSON: &str = r#"{"type":"webauthn.get","challenge":"","origin":""}"#;
            const AUTH_DATA_SIZE: usize = 37;
            const MIN_SIZE: usize = AUTH_DATA_SIZE + CLIENT_JSON.len();
            const DEFAULT_SIZE: usize = 800;
            const MAX_SIZE: usize = 8192;

            let size = key_data
                .as_deref()
                .and_then(|data| match data.len() {
                    1 => Some(data[0] as usize),
                    2 => Some(u16::from_be_bytes([data[0], data[1]]) as usize),
                    4 => Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize),
                    _ => None,
                })
                .unwrap_or(DEFAULT_SIZE)
                .clamp(MIN_SIZE, MAX_SIZE);
            let mut webauthn_data = vec![0u8; AUTH_DATA_SIZE];
            webauthn_data[32] = 0x01;
            let padding = "x".repeat(size - MIN_SIZE);
            webauthn_data.extend_from_slice(
                format!(r#"{{"type":"webauthn.get","challenge":"","origin":"{padding}"}}"#)
                    .as_bytes(),
            );
            PrimitiveSignature::WebAuthn(WebAuthnSignature {
                webauthn_data: webauthn_data.into(),
                r: B256::ZERO,
                s: B256::ZERO,
                pub_key_x: B256::ZERO,
                pub_key_y: B256::ZERO,
            })
        }
    };

    if key_id.is_some() {
        let signature = if is_t1c {
            KeychainSignature::new(caller, signature)
        } else {
            KeychainSignature::new_v1(caller, signature)
        };
        TempoSignature::Keychain(signature)
    } else {
        TempoSignature::Primitive(signature)
    }
}

fn call_config_from_tracer_config(
    tracer_config: GethDebugTracerConfig,
) -> Result<CallConfig, serde_json::Error> {
    let mut tracer_config = tracer_config.into_json();
    if let Some(config) = tracer_config.as_object_mut()
        && !config.contains_key("onlyTopCall")
        && let Some(only_top_level_call) = config.remove("onlyTopLevelCall")
    {
        config.insert("onlyTopCall".to_string(), only_top_level_call);
    }

    GethDebugTracerConfig(tracer_config).into_call_config()
}

/// Builds Parity trace results for `result`, with `db` as the state before the transaction.
///
/// The state diff treats empty accounts as absent, so created and newly funded accounts are
/// marked as added.
fn parity_trace_results(
    inspector: TracingInspector,
    executed: &Executed,
    trace_types: &HashSet<TraceType>,
    db: &dyn DatabaseRef,
    spec: SpecId,
) -> Result<TraceResults, BlockchainError> {
    Ok(inspector.into_parity_builder().into_trace_results_with_state_parts(
        &executed.result,
        &executed.state,
        trace_types,
        &mut EvmDb::new(db, spec, None),
    )?)
}

/// Builds a geth prestate trace for `executed`, with `db` as the state before the transaction.
fn geth_prestate_traces(
    inspector: TracingInspector,
    executed: Executed,
    config: &alloy_rpc_types::trace::geth::PreStateConfig,
    db: &dyn DatabaseRef,
    spec: SpecId,
) -> Result<alloy_rpc_types::trace::geth::PreStateFrame, BlockchainError> {
    let detached = evm2::evm::TxResultWithState::<evm2::BaseEvmTypes> {
        result: executed.result.map_ext(|_| ()),
        pending_state: executed.state,
        ..Default::default()
    };
    Ok(inspector.into_geth_builder().geth_prestate_traces(
        &detached,
        config,
        &mut EvmDb::new(db, spec, None),
    )?)
}

/// Returns the error of a JavaScript tracer request when the feature is disabled.
#[cfg(not(feature = "js-tracer"))]
fn js_tracer_unsupported() -> BlockchainError {
    RpcError::invalid_params("JavaScript tracers are not supported by this build of anvil").into()
}

/// Collects a JavaScript trace over the same network transaction and backing state that ran.
#[cfg(feature = "js-tracer")]
fn js_trace_result<E: AnvilEvmNetwork>(
    inspector: &mut JsInspector,
    executed: Executed,
    tx: &PreparedTx,
    env: &EvmEnv,
    db: &dyn DatabaseRef,
) -> Result<GethTrace, BlockchainError>
where
    TxFor<E>: TransactionTrait,
{
    let result = evm2::evm::TxResultWithState::<TypesFor<E>> {
        result: executed.result.map_ext(|_| Default::default()),
        pending_state: executed.state,
        ..Default::default()
    };
    let tx = E::prepared_evm_tx(tx, None)?;
    inspector
        .json_result(
            &result,
            &tx,
            &block_env::<TypesFor<E>>(env),
            &mut EvmDb::new(db, *env.spec_id(), None),
        )
        .map(GethTrace::JS)
        .map_err(|err| BlockchainError::Message(err.to_string()))
}

/// Rejects the `pending` tag for block trace methods, which only trace mined blocks.
///
/// Resolving it to the latest block would present mined transactions as the pending block.
fn ensure_mined_trace_block(block: BlockNumber) -> Result<(), BlockchainError> {
    if block.is_pending() {
        return Err(BlockchainError::RpcError(RpcError::invalid_params(
            "pending block traces are not supported",
        )));
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct SimulationPrecompileOverrides {
    moves: Vec<(Address, Address)>,
}

/// A block request, which includes the Pool Transactions if it's Pending
pub enum BlockRequest<T> {
    Pending(Vec<Arc<PoolTransaction<T>>>),
    Number(u64),
}

impl<T> fmt::Debug for BlockRequest<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending(txs) => f.debug_tuple("Pending").field(&txs.len()).finish(),
            Self::Number(n) => f.debug_tuple("Number").field(n).finish(),
        }
    }
}

impl<T> BlockRequest<T> {
    pub const fn block_number(&self) -> BlockNumber {
        match *self {
            Self::Pending(_) => BlockNumber::Pending,
            Self::Number(n) => BlockNumber::Number(n),
        }
    }
}

struct StateSnapshot {
    block_number: u64,
    block_hash: B256,
    fees: FeeSnapshot,
    time: TimeSnapshot,
    next_block: NextBlockOverrides,
}

#[cfg(test)]
#[derive(Clone)]
struct MiningCommitHook {
    reached: Arc<Notify>,
    resume: Arc<Notify>,
}

#[cfg(test)]
type StorageAccessHook = Box<dyn Fn() + Send + Sync>;

/// Owns the chain state and executes transactions on evm2.
pub struct Backend<N: Network> {
    /// Access to Anvil's [`Db`] abstraction.
    ///
    /// [`EvmDb`] serves reads to [`evm2::Evm`]; committed changes are applied to this database.
    ///
    /// At time of writing, there are two different types of `Db`:
    ///   - [`MemDb`](crate::mem::in_memory_db::MemDb): everything is stored in memory
    ///   - [`ForkDb`](crate::mem::fork_db::ForkedDatabase): forks off a remote client, missing
    ///     data is retrieved via RPC-calls
    ///
    /// Committing changes requires mutable access and a write-lock on this `db`. In forking mode,
    /// the time during which the write-lock is active depends on whether the `ForkDb` can
    /// provide all requested data from memory or whether it has to retrieve it via RPC calls
    /// first. This means that it potentially blocks for some time, even taking into account
    /// the rate limits of RPC endpoints. Therefore the `Db` is guarded by a
    /// `tokio::sync::RwLock` here so calls that need to read from it, while it's currently
    /// written to, don't block. E.g. a new block is currently mined and a new
    /// [`Self::set_storage_at()`] request is being executed.
    db: Arc<AsyncRwLock<Box<dyn Db>>>,
    /// stores all block related data in memory.
    blockchain: Blockchain<N>,
    /// Historic states of previous blocks.
    states: Arc<RwLock<InMemoryBlockStates>>,
    /// EVM environment data of the chain (block env, cfg env).
    evm_env: Arc<RwLock<EvmEnv>>,
    /// Network configuration (optimism, custom precompiles, etc.)
    networks: NetworkConfigs,
    /// Base activation-registry administrator override.
    #[cfg(feature = "base")]
    base_activation_admin: Option<Address>,
    /// The active hardfork.
    hardfork: Arc<RwLock<FoundryHardfork>>,
    /// This is set if this is currently forked off another client.
    fork: Arc<RwLock<Option<ClientFork>>>,
    /// The last source that supplied the live fork backend, retained across memory resets.
    last_fork_cache_source: Arc<RwLock<Option<ForkCacheSource>>>,
    /// Provides time related info, like timestamp.
    time: TimeManager,
    /// Contains state of custom overrides.
    cheats: CheatsManager,
    /// Contains fee data.
    fees: FeeManager,
    /// Initialised genesis.
    genesis: GenesisConfig,
    /// Listeners for new blocks that get notified when a new block was imported or when logs were
    /// removed from the canonical chain due to a reorg.
    new_block_listeners: Arc<Mutex<Vec<UnboundedSender<ChainNotification>>>>,
    /// Keeps track of active state snapshots at a specific block.
    active_state_snapshots: Arc<Mutex<HashMap<U256, StateSnapshot>>>,
    enable_steps_tracing: bool,
    print_logs: bool,
    print_traces: bool,
    /// Recorder used for decoding traces, used together with print_traces.
    call_trace_decoder: Arc<RwLock<Arc<CallTraceDecoder>>>,
    /// How to keep history state
    prune_state_history_config: PruneStateHistoryConfig,
    /// max number of blocks with transactions in memory
    transaction_block_keeper: Option<usize>,
    pub(crate) node_config: Arc<AsyncRwLock<NodeConfig>>,
    /// Slots in an epoch
    slots_in_an_epoch: u64,
    /// Precompiles to inject to the EVM.
    precompile_factory: Option<Arc<dyn PrecompileFactory>>,
    /// Prevent race conditions during mining
    mining: Arc<tokio::sync::Mutex<()>>,
    /// Test-only synchronization point after database commit and before canonical publication.
    #[cfg(test)]
    mining_commit_hook: Arc<Mutex<Option<MiningCommitHook>>>,
    /// Test-only observer immediately before the mined state root is computed.
    #[cfg(test)]
    mining_state_root_hook: Arc<Mutex<Option<StorageAccessHook>>>,
    /// Test-only observer between consensus and canonical rollback updates.
    #[cfg(test)]
    rollback_unwind_hook: Arc<Mutex<Option<StorageAccessHook>>>,
    /// Disable pool balance checks
    disable_pool_balance_checks: bool,
    /// Keeps startup fork-cache rollback armed until startup initialization completes.
    ///
    /// This must remain the final field so all other backend-held database references are released
    /// before a rejected startup invalidates its cache.
    startup_fork_cache_user: StagedForkDbUser<Box<dyn Db>>,
}

impl<N: Network> Clone for Backend<N> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            blockchain: self.blockchain.clone(),
            states: self.states.clone(),
            evm_env: self.evm_env.clone(),
            networks: self.networks,
            #[cfg(feature = "base")]
            base_activation_admin: self.base_activation_admin,
            hardfork: self.hardfork.clone(),
            fork: self.fork.clone(),
            last_fork_cache_source: self.last_fork_cache_source.clone(),
            time: self.time.clone(),
            cheats: self.cheats.clone(),
            fees: self.fees.clone(),
            genesis: self.genesis.clone(),
            new_block_listeners: self.new_block_listeners.clone(),
            active_state_snapshots: self.active_state_snapshots.clone(),
            enable_steps_tracing: self.enable_steps_tracing,
            print_logs: self.print_logs,
            print_traces: self.print_traces,
            call_trace_decoder: self.call_trace_decoder.clone(),
            prune_state_history_config: self.prune_state_history_config,
            transaction_block_keeper: self.transaction_block_keeper,
            node_config: self.node_config.clone(),
            slots_in_an_epoch: self.slots_in_an_epoch,
            precompile_factory: self.precompile_factory.clone(),
            mining: self.mining.clone(),
            #[cfg(test)]
            mining_commit_hook: self.mining_commit_hook.clone(),
            #[cfg(test)]
            mining_state_root_hook: self.mining_state_root_hook.clone(),
            #[cfg(test)]
            rollback_unwind_hook: self.rollback_unwind_hook.clone(),
            disable_pool_balance_checks: self.disable_pool_balance_checks,
            startup_fork_cache_user: self.startup_fork_cache_user.clone(),
        }
    }
}

impl<N: Network> fmt::Debug for Backend<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backend").finish_non_exhaustive()
    }
}

// Methods that are generic over any Network.
impl<N: Network> Backend<N> {
    /// Sets the account to impersonate
    ///
    /// Returns `true` if the account is already impersonated
    pub fn impersonate(&self, addr: Address) -> bool {
        if self.cheats.impersonated_accounts().contains(&addr) {
            return true;
        }
        // Ensure EIP-3607 is disabled
        self.evm_env.write().cfg_env.disable_eip3607 = true;
        self.cheats.impersonate(addr)
    }

    /// Removes the account that from the impersonated set
    ///
    /// If the impersonated `addr` is a contract then we also reset the code here
    pub fn stop_impersonating(&self, addr: Address) {
        self.cheats.stop_impersonating(&addr);
    }

    /// If set to true will make every account impersonated
    pub fn auto_impersonate_account(&self, enabled: bool) {
        self.cheats.set_auto_impersonate_account(enabled);
    }

    /// Returns the configured fork, if any
    pub fn get_fork(&self) -> Option<ClientFork> {
        self.fork.read().clone()
    }

    /// Marks startup fork-cache writes as belonging to the validated live backend.
    pub(crate) fn commit_startup_fork_cache(&self) {
        self.startup_fork_cache_user.cache_lease.disarm();
    }

    /// Returns the database
    pub fn get_db(&self) -> &Arc<AsyncRwLock<Box<dyn Db>>> {
        &self.db
    }

    /// Locks block production while a backend-wide lifecycle transition is committed.
    pub(crate) async fn lock_mining(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.mining.lock().await
    }

    /// Locks block production with a guard that can be moved into a mining task.
    pub(crate) async fn lock_mining_owned(self: &Arc<Self>) -> tokio::sync::OwnedMutexGuard<()> {
        self.mining.clone().lock_owned().await
    }

    /// Returns the `AccountInfo` from the database
    pub async fn get_account(&self, address: Address) -> DatabaseResult<AccountInfo> {
        Ok(self.db.read().await.basic_ref(address)?.unwrap_or_default())
    }

    /// Whether we're forked off some remote client
    pub fn is_fork(&self) -> bool {
        self.fork.read().is_some()
    }

    /// Writes the CREATE2 deployer code directly to the database at the address provided.
    pub async fn set_create2_deployer(&self, address: Address) -> DatabaseResult<()> {
        self.set_code(address, Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE)).await?;
        Ok(())
    }

    /// Updates memory limits that should be more strict when auto-mine is enabled
    pub(crate) fn update_interval_mine_block_time(&self, block_time: Duration) {
        self.states.write().update_interval_mine_block_time(block_time);
        if let Some(states) = self.tempo_proof_states() {
            states.write().update_interval_mine_block_time(block_time);
        }
    }

    /// Returns the `TimeManager` responsible for timestamps
    pub const fn time(&self) -> &TimeManager {
        &self.time
    }

    /// Returns the `CheatsManager` responsible for executing cheatcodes
    pub const fn cheats(&self) -> &CheatsManager {
        &self.cheats
    }

    /// Whether to skip blob validation
    pub fn skip_blob_validation(&self, impersonator: Option<Address>) -> bool {
        self.cheats().auto_impersonate_accounts()
            || impersonator
                .is_some_and(|addr| self.cheats().impersonated_accounts().contains(&addr))
    }

    /// Returns the `FeeManager` that manages fee/pricings
    pub const fn fees(&self) -> &FeeManager {
        &self.fees
    }

    /// The EVM environment data of the blockchain
    pub const fn evm_env(&self) -> &Arc<RwLock<EvmEnv>> {
        &self.evm_env
    }

    /// Returns the current best hash of the chain
    pub fn best_hash(&self) -> B256 {
        self.blockchain.storage.read().best_hash
    }

    /// Returns the current best number of the chain
    pub fn best_number(&self) -> u64 {
        self.blockchain.storage.read().best_number
    }

    /// Returns the client coinbase address.
    pub fn coinbase(&self) -> Address {
        self.evm_env.read().block_env.beneficiary
    }

    /// Returns the client coinbase address.
    pub fn chain_id(&self) -> U256 {
        U256::from(self.evm_env.read().cfg_env.chain_id)
    }

    /// Returns the chain ID that defines protocol behavior.
    fn protocol_chain_id(&self) -> u64 {
        self.get_fork().map_or_else(|| self.evm_env.read().cfg_env.chain_id, |fork| fork.chain_id())
    }

    pub fn set_chain_id(&self, chain_id: u64) {
        self.evm_env.write().cfg_env.chain_id = chain_id;
    }

    /// Returns the genesis data for the Beacon API.
    pub const fn genesis_time(&self) -> u64 {
        self.genesis.timestamp
    }

    /// Returns the configured genesis block number.
    pub const fn genesis_number(&self) -> u64 {
        self.genesis.number
    }

    /// Returns balance of the given account.
    pub async fn current_balance(&self, address: Address) -> DatabaseResult<U256> {
        Ok(self.get_account(address).await?.balance)
    }

    /// Returns balance of the given account.
    pub async fn current_nonce(&self, address: Address) -> DatabaseResult<u64> {
        Ok(self.get_account(address).await?.nonce)
    }

    /// Sets the coinbase address
    pub fn set_coinbase(&self, address: Address) {
        self.evm_env.write().block_env.beneficiary = address;
    }

    /// Sets the `prevrandao` value to use for the next mined block.
    ///
    /// This is a one-shot override that is consumed by the next block; afterwards anvil resumes its
    /// default per-block `prevrandao` derivation.
    pub fn set_next_block_prevrandao(&self, prevrandao: B256) {
        self.cheats.set_next_block_prevrandao(prevrandao);
    }

    /// Sets the parent beacon block root to use for the next mined block.
    ///
    /// This is a one-shot override that is consumed by the next block; afterwards anvil resumes
    /// using the zero root.
    pub fn set_next_block_parent_beacon_block_root(&self, root: B256) {
        self.cheats.set_next_block_parent_beacon_block_root(root);
    }

    /// Sets the nonce of the given address
    pub async fn set_nonce(&self, address: Address, nonce: U256) -> DatabaseResult<()> {
        self.db.write().await.set_nonce(address, nonce.try_into().unwrap_or(u64::MAX))
    }

    /// Sets the balance of the given address
    pub async fn set_balance(&self, address: Address, balance: U256) -> DatabaseResult<()> {
        self.db.write().await.set_balance(address, balance)
    }

    /// Increases the balance of the given address, saturating at `U256::MAX`.
    pub(crate) async fn add_balance(&self, address: Address, balance: U256) -> DatabaseResult<()> {
        let mut db = self.db.write().await;
        let current_balance = db.basic_ref(address)?.unwrap_or_default().balance;
        db.set_balance(address, current_balance.saturating_add(balance))
    }

    /// Sets the code of the given address
    pub async fn set_code(&self, address: Address, code: Bytes) -> DatabaseResult<()> {
        self.db.write().await.set_code(address, code)
    }

    /// Sets the value for the given slot of the given address
    pub async fn set_storage_at(
        &self,
        address: Address,
        slot: U256,
        val: B256,
    ) -> DatabaseResult<()> {
        self.db.write().await.set_storage_at(address, slot.into(), val)
    }

    /// Returns the configured specid
    pub fn spec_id(&self) -> SpecId {
        *self.evm_env.read().spec_id()
    }

    /// Returns true for post London
    pub fn is_eip1559(&self) -> bool {
        self.spec_id().enables(SpecId::LONDON)
    }

    /// Returns true for post Merge
    pub fn is_eip3675(&self) -> bool {
        self.spec_id().enables(SpecId::MERGE)
    }

    /// Returns true for post Berlin
    pub fn is_eip2930(&self) -> bool {
        self.spec_id().enables(SpecId::BERLIN)
    }

    /// Returns true for post Cancun
    pub fn is_eip4844(&self) -> bool {
        self.spec_id().enables(SpecId::CANCUN)
    }

    /// Returns true for post Prague
    pub fn is_eip7702(&self) -> bool {
        self.spec_id().enables(SpecId::PRAGUE)
    }

    /// Returns true if op-stack deposits are active.
    ///
    /// Always `false` when built without the `optimism` feature.
    pub const fn is_optimism(&self) -> bool {
        self.networks.is_optimism()
    }

    /// Returns true if Tempo network mode is active
    pub const fn is_tempo(&self) -> bool {
        self.networks.is_tempo()
    }

    /// Returns whether Celo compatibility is enabled.
    pub const fn is_celo(&self) -> bool {
        self.networks.is_celo()
    }

    /// Returns true if Monad network mode is active
    pub const fn is_monad(&self) -> bool {
        self.networks.is_monad()
    }

    /// Returns the active execution profile name.
    pub const fn execution_profile_name(&self) -> &'static str {
        self.networks.execution_profile_name()
    }

    /// Reconstructs a locally mined transaction using its authoritative stored sender.
    fn pending_mined_transaction(
        &self,
        transaction: MaybeImpersonatedTransaction<FoundryTxEnvelope>,
    ) -> Result<PendingTransaction<FoundryTxEnvelope>, BlockchainError> {
        #[cfg(feature = "monad")]
        if self.is_monad() {
            return Self::monad_pending_mined_transaction_from_storage(
                &self.blockchain.storage.read(),
                transaction,
            );
        }
        Ok(PendingTransaction::from_maybe_impersonated(transaction)?)
    }

    #[cfg(not(feature = "monad"))]
    fn active_monad_context_for_mined_block(
        &self,
        _block: &Block,
    ) -> Result<Option<MonadReplayContext>, BlockchainError> {
        Ok(None)
    }

    #[cfg(not(feature = "monad"))]
    fn active_monad_context_before_mined_transaction(
        &self,
        _block: &Block,
        _current_tx_index: usize,
    ) -> Result<Option<MonadReplayContext>, BlockchainError> {
        Ok(None)
    }

    /// Returns true if Base network mode is active.
    #[cfg(feature = "base")]
    pub const fn is_base(&self) -> bool {
        self.networks.is_base()
    }

    /// Returns the active hardfork.
    pub fn hardfork(&self) -> FoundryHardfork {
        if let Some(hardfork) =
            self.fork.read().as_ref().and_then(|fork| fork.config.read().hardfork)
        {
            return hardfork;
        }
        *self.hardfork.read()
    }

    /// Returns canonical Ethereum transition configuration only for an Ethereum network.
    fn ethereum_block_transitions(
        &self,
        hardfork: FoundryHardfork,
        parent_beacon_block_root: Option<B256>,
        execution_kind: BlockExecutionKind,
    ) -> Option<EthereumBlockTransitions> {
        if self.is_optimism() || self.is_tempo() {
            return None;
        }
        let FoundryHardfork::Ethereum(hardfork) = hardfork else { return None };
        Some(EthereumBlockTransitions {
            hardfork,
            deposit_contract_address: self.ethereum_deposit_contract_address(),
            parent_beacon_block_root,
            execution_kind,
        })
    }

    /// Returns the configured deposit contract, then the canonical address for known chains.
    fn ethereum_deposit_contract_address(&self) -> Address {
        if let Some(address) = self
            .genesis
            .genesis_init
            .as_ref()
            .and_then(|genesis| genesis.config.deposit_contract_address)
        {
            return address;
        }

        match NamedChain::try_from(self.evm_env.read().cfg_env.chain_id) {
            Ok(NamedChain::Sepolia) => SEPOLIA_DEPOSIT_CONTRACT_ADDRESS,
            Ok(NamedChain::Holesky) => HOLESKY_DEPOSIT_CONTRACT_ADDRESS,
            // Hoodi shares the mainnet address; other chains use Alloy's mainnet fallback.
            _ => MAINNET_DEPOSIT_CONTRACT_ADDRESS,
        }
    }

    /// Returns the active Tempo hardfork.
    pub fn tempo_hardfork(&self) -> TempoHardfork {
        TempoHardfork::from(self.hardfork())
    }

    /// Returns the active Monad hardfork.
    #[cfg(feature = "monad")]
    pub fn monad_hardfork(&self) -> foundry_evm_hardforks::MonadHardfork {
        foundry_evm_hardforks::MonadHardfork::from(self.hardfork())
    }

    /// Returns the active Base upgrade.
    #[cfg(feature = "base")]
    pub fn base_upgrade(&self) -> BaseUpgrade {
        match self.hardfork() {
            FoundryHardfork::Base(upgrade) => upgrade,
            _ => BaseUpgrade::Azul,
        }
    }

    /// Returns the configured Base activation-registry administrator override.
    #[cfg(feature = "base")]
    pub const fn base_activation_admin(&self) -> Option<Address> {
        self.base_activation_admin
    }

    /// Returns the Base upgrade active at `timestamp`.
    #[cfg(feature = "base")]
    pub fn base_upgrade_at_timestamp(&self, timestamp: u64) -> BaseUpgrade {
        if self.is_fork() {
            BaseUpgrade::from_chain_and_timestamp(self.chain_id().to(), timestamp)
                .unwrap_or_else(|| self.base_upgrade())
        } else {
            self.base_upgrade()
        }
    }

    /// Prepares the clock for a candidate block without consuming its time controls.
    fn prepare_block_timestamp(
        &self,
        db: &dyn DatabaseRef,
        parent_timestamp: u64,
        pin_timestamp: bool,
    ) -> Result<PendingBlockTimestamp, DatabaseError> {
        #[cfg(feature = "base")]
        if self.is_base() && self.base_upgrade() >= BaseUpgrade::Denim {
            return base::prepare_block_timestamp(db, &self.time, parent_timestamp);
        }
        let _ = (db, parent_timestamp);
        Ok(self.time.prepare_next_timestamp(pin_timestamp))
    }

    /// Builds the required deposits from the same parent state used to prepare the clock.
    #[cfg(feature = "base")]
    fn base_system_transactions(
        &self,
        db: &dyn DatabaseRef,
        block_number: u64,
        timestamp: u64,
        parent_hash: B256,
    ) -> Result<Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>, DatabaseError> {
        if self.is_base() && self.base_upgrade() >= BaseUpgrade::Denim {
            base::system_transactions(db, block_number, timestamp, parent_hash, self.is_fork())
        } else {
            Ok(Vec::new())
        }
    }

    /// Returns the head timestamp snapshot used for EIP-8130 pool admission.
    #[cfg(feature = "base")]
    pub fn eip8130_pool_timestamp(&self) -> u64 {
        self.evm_env.read().block_env.timestamp.saturating_to()
    }

    /// Returns the EIP-8130 pool admission snapshot in Unix milliseconds.
    ///
    /// EIP-8130 validity windows are millisecond timestamps evaluated against
    /// `block.timestamp * 1000`, so protocol validation must use this rather than the
    /// second-valued block timestamp.
    #[cfg(feature = "base")]
    pub fn eip8130_pool_timestamp_ms(&self) -> u64 {
        self.eip8130_pool_timestamp().saturating_mul(1_000)
    }

    /// Returns whether a Tempo hardfork is active on this backend.
    pub fn is_tempo_hardfork_active(&self, hardfork: TempoHardfork) -> bool {
        self.is_tempo() && self.tempo_hardfork() >= hardfork
    }

    /// Returns the precompiles for the current spec.
    pub fn precompiles(&self) -> BTreeMap<String, Address> {
        let evm_env = self.evm_env.read().clone();
        let opts = self.evm_options(&evm_env);
        let db = crate::eth::backend::db::EmptyDb;
        let ids = with_evm_network!(self.evm_family(), E => {
            build_evm::<E>(&db, &evm_env, &opts).map(|evm| evm.precompiles().precompile_ids())
        })
        .unwrap_or_default();
        let mut precompiles_map = ids
            .into_iter()
            .map(|(address, id)| (id.name().to_string(), address))
            .collect::<BTreeMap<_, _>>();

        // Extend with configured network precompiles.
        precompiles_map.extend(self.networks.precompiles(Some(self.hardfork())));
        precompiles_map
    }

    /// Returns whether the address is an active precompile in the given block environment.
    pub fn is_precompile(&self, address: &Address, block_env: &BlockEnv) -> bool {
        let mut evm_env = self.evm_env.read().clone();
        evm_env.block_env = block_env.clone();
        self.precompile_addresses(&evm_env).is_ok_and(|addresses| addresses.contains(address))
    }

    /// Returns the system contracts for the current spec.
    pub fn system_contracts(&self) -> BTreeMap<SystemContract, Address> {
        let mut system_contracts = BTreeMap::<SystemContract, Address>::default();

        let spec_id = self.spec_id();

        if spec_id >= SpecId::CANCUN {
            system_contracts.extend(SystemContract::cancun());
        }

        if spec_id >= SpecId::PRAGUE {
            system_contracts.extend(SystemContract::prague(None));
        }

        system_contracts
    }

    /// Returns the active [`BlobParams`].
    pub fn blob_params(&self) -> BlobParams {
        self.fees.blob_params()
    }

    fn simulation_blob_params_at_timestamp(&self, timestamp: u64) -> BlobParams {
        let configured_hardfork = self.hardfork();
        if let FoundryHardfork::Ethereum(
            configured
            @ (EthereumHardfork::Osaka | EthereumHardfork::Bpo1 | EthereumHardfork::Bpo2),
        ) = configured_hardfork
            && let Some(hardfork) = FoundryHardfork::from_chain_and_timestamp(
                self.evm_env.read().cfg_env.chain_id,
                timestamp,
            )
            && let FoundryHardfork::Ethereum(
                scheduled @ (EthereumHardfork::Osaka
                | EthereumHardfork::Bpo1
                | EthereumHardfork::Bpo2),
            ) = hardfork
        {
            let hardfork = match (configured, scheduled) {
                (EthereumHardfork::Bpo2, _) | (_, EthereumHardfork::Bpo2) => EthereumHardfork::Bpo2,
                (EthereumHardfork::Bpo1, _) | (_, EthereumHardfork::Bpo1) => EthereumHardfork::Bpo1,
                _ => EthereumHardfork::Osaka,
            };
            return get_blob_params_by_hardfork(hardfork.into());
        }
        get_blob_params_by_hardfork(configured_hardfork)
    }

    #[cfg(feature = "optimism")]
    fn is_optimism_jovian_at_header<H: BlockHeader>(
        &self,
        header: &H,
        decoded: Option<bool>,
    ) -> bool {
        if !self.is_optimism() {
            return false;
        }
        if let Some(jovian) = decoded {
            return jovian;
        }
        if !header.extra_data().is_empty() {
            return false;
        }
        let hardfork = if self.get_fork().is_some() {
            NetworkVariant::Optimism
                .historical_hardfork(self.protocol_chain_id(), header.timestamp())
                .unwrap_or_else(|| self.hardfork())
        } else {
            self.hardfork()
        };
        OpHardfork::from(hardfork) >= OpHardfork::Jovian
    }

    #[cfg(not(feature = "optimism"))]
    const fn is_optimism_jovian_at_header<H: BlockHeader>(
        &self,
        _header: &H,
        _decoded: Option<bool>,
    ) -> bool {
        false
    }

    /// Returns an error if EIP1559 is not active (pre Berlin)
    pub fn ensure_eip1559_active(&self) -> Result<(), BlockchainError> {
        if self.is_eip1559() {
            return Ok(());
        }
        Err(BlockchainError::EIP1559TransactionUnsupportedAtHardfork)
    }

    /// Returns an error if EIP1559 is not active (pre muirGlacier)
    pub fn ensure_eip2930_active(&self) -> Result<(), BlockchainError> {
        if self.is_eip2930() {
            return Ok(());
        }
        Err(BlockchainError::EIP2930TransactionUnsupportedAtHardfork)
    }

    pub fn ensure_eip4844_active(&self) -> Result<(), BlockchainError> {
        if self.is_eip4844() {
            return Ok(());
        }
        Err(BlockchainError::EIP4844TransactionUnsupportedAtHardfork)
    }

    pub fn ensure_eip7702_active(&self) -> Result<(), BlockchainError> {
        if self.is_eip7702() {
            return Ok(());
        }
        Err(BlockchainError::EIP7702TransactionUnsupportedAtHardfork)
    }

    /// Returns an error if deposit transactions are not active.
    #[cfg(any(feature = "base", feature = "optimism"))]
    pub const fn ensure_deposits_active(&self) -> Result<(), BlockchainError> {
        #[cfg(feature = "base")]
        if self.is_base() {
            return Ok(());
        }
        #[cfg(feature = "optimism")]
        if self.is_optimism() {
            return Ok(());
        }
        Err(BlockchainError::DepositTransactionUnsupported)
    }

    /// Returns an error if Base EIP-8130 RPC features are not active at `timestamp`.
    #[cfg(feature = "base")]
    pub fn ensure_base_eip8130_active_at(&self, timestamp: u64) -> Result<(), BlockchainError> {
        let upgrade = self.base_upgrade_at_timestamp(timestamp);
        if self.is_base() && upgrade >= BaseUpgrade::Zenith {
            return Ok(());
        }
        Err(BlockchainError::InvalidTransactionRequest(EIP8130_PRE_ZENITH_RPC_ERROR.to_string()))
    }

    /// Returns an admission error if Base EIP-8130 submissions are not active.
    #[cfg(feature = "base")]
    pub fn ensure_base_eip8130_submission_active(&self) -> Result<(), BlockchainError> {
        if self.is_base() && self.base_upgrade() >= BaseUpgrade::Zenith {
            return Ok(());
        }
        Err(BlockchainError::Eip8130TransactionRejected(EIP8130_REJECTION_MSG.to_string()))
    }

    /// Returns an error if Tempo transactions are not active
    pub const fn ensure_tempo_active(&self) -> Result<(), BlockchainError> {
        if self.is_tempo() {
            return Ok(());
        }
        Err(BlockchainError::TempoTransactionUnsupported)
    }

    /// Returns an error unless CIP-64 native-fee compatibility is active.
    pub fn ensure_cip64_active(&self) -> Result<(), BlockchainError> {
        if !self.is_celo() {
            return Err(BlockchainError::InvalidTransactionRequest(
                "CIP-64 transactions require Celo mode (--celo)".into(),
            ));
        }
        self.ensure_eip1559_active()
    }

    /// Builds the [`InspectorTxConfig`] from the backend's current settings.
    fn inspector_tx_config(&self) -> InspectorTxConfig {
        InspectorTxConfig {
            print_traces: self.print_traces,
            print_logs: self.print_logs,
            call_trace_decoder: self.call_trace_decoder(),
        }
    }

    /// Returns a trace decoder configured for the currently resolved hardfork.
    fn call_trace_decoder(&self) -> Arc<CallTraceDecoder> {
        let hardfork = Some(self.networks.executed_hardfork(self.hardfork()));
        let decoder = self.call_trace_decoder.read();
        if decoder.hardfork() == hardfork {
            return Arc::clone(&decoder);
        }
        drop(decoder);

        let mut decoder = self.call_trace_decoder.write();
        let mut updated = decoder.as_ref().clone();
        updated.set_hardfork(hardfork);
        *decoder = Arc::new(updated);
        Arc::clone(&decoder)
    }

    /// Builds the [`PoolTxGasConfig`] from the given EVM environment.
    fn pool_tx_gas_config(&self, evm_env: &EvmEnv) -> PoolTxGasConfig {
        let spec_id = *evm_env.spec_id();
        let is_cancun = spec_id >= SpecId::CANCUN;
        let blob_params = self.blob_params();
        PoolTxGasConfig {
            disable_block_gas_limit: evm_env.cfg_env.disable_block_gas_limit,
            enforced_tx_gas_limit_cap: self.enforced_tx_gas_limit_cap(evm_env),
            max_blob_gas_per_block: blob_params.max_blob_gas_per_block(),
            is_cancun,
        }
    }

    /// Returns the EIP-7825 cap enforced on a transaction's gas limit, if any.
    ///
    /// Under EIP-8037 the cap only bounds regular gas, and gas above it becomes the state gas
    /// reservoir, so the gas limit itself is not capped.
    fn enforced_tx_gas_limit_cap(&self, evm_env: &EvmEnv) -> Option<u64> {
        if evm_env.cfg_env.tx_gas_limit_cap.is_some() || evm_env.cfg_env.spec >= SpecId::AMSTERDAM {
            return None;
        }
        #[cfg(feature = "monad")]
        if self.is_monad() {
            return Some(monad_evm2::MONAD_TX_GAS_LIMIT_CAP);
        }
        Some(evm_env.cfg_env.tx_gas_limit_cap())
    }

    pub(crate) fn fallback_tx_gas_limit(&self, evm_env: &EvmEnv) -> u64 {
        let block_gas_limit = evm_env.block_env.gas_limit;
        self.enforced_tx_gas_limit_cap(evm_env)
            .map_or(block_gas_limit, |cap| block_gas_limit.min(cap))
    }

    fn max_initcode_size(&self, evm_env: &EvmEnv) -> usize {
        #[cfg(feature = "monad")]
        if self.is_monad() {
            return monad_evm2::MONAD_MAX_INITCODE_SIZE;
        }
        evm_env.cfg_env.max_initcode_size()
    }

    /// Returns the block gas limit
    pub fn gas_limit(&self) -> u64 {
        self.evm_env.read().block_env.gas_limit
    }

    /// Sets the block gas limit
    pub fn set_gas_limit(&self, gas_limit: u64) {
        self.evm_env.write().block_env.gas_limit = gas_limit;
    }

    /// Returns the current base fee
    pub fn base_fee(&self) -> u64 {
        self.fees.base_fee()
    }

    /// Returns whether the minimum suggested priority fee is enforced
    pub const fn is_min_priority_fee_enforced(&self) -> bool {
        self.fees.is_min_priority_fee_enforced()
    }

    pub fn excess_blob_gas_and_price(&self) -> Option<BlobExcessGasAndPrice> {
        self.fees.excess_blob_gas_and_price()
    }

    /// Sets the current basefee
    pub fn set_base_fee(&self, basefee: u64) {
        self.fees.set_base_fee(basefee)
    }

    /// Sets the gas price
    pub fn set_gas_price(&self, price: u128) {
        self.fees.set_gas_price(price)
    }

    pub fn elasticity(&self) -> f64 {
        self.fees.elasticity()
    }

    /// Returns the total difficulty of the chain until this block
    ///
    /// Note: this will always be `0` in memory mode
    /// In forking mode this will always be the total difficulty of the forked block
    pub fn total_difficulty(&self) -> U256 {
        self.blockchain.storage.read().total_difficulty
    }

    /// Creates a new `evm_snapshot` at the current height.
    ///
    /// Returns the id of the snapshot created.
    pub async fn create_state_snapshot(&self) -> U256 {
        let mut db = self.db.write().await;
        let num = self.best_number();
        let hash = self.best_hash();
        let id = db.snapshot_state();
        let proof_state = self.tempo_proof_states().and_then(|states| {
            // Explicit snapshots keep the sealed head independently of bounded proof history.
            // Live execution may already contain unmined development edits.
            let states = states.upgradable_read();
            if let Some(state) = states.get_state(&hash) {
                return state.maybe_as_full_db().cloned().map(Arc::new);
            }
            let mut states = RwLockUpgradableReadGuard::upgrade(states);
            states
                .get_on_disk_state(&hash)
                .and_then(|state| state.maybe_as_full_db().cloned())
                .map(Arc::new)
        });
        if let Some(tempo) = &mut self.blockchain.storage.write().tempo {
            tempo.snapshot(id, proof_state);
        }
        trace!(target: "backend", "creating snapshot {} at {}", id, num);
        self.active_state_snapshots.lock().insert(
            id,
            StateSnapshot {
                block_number: num,
                block_hash: hash,
                fees: self.fees.snapshot(),
                time: self.time.snapshot(),
                next_block: self.cheats.next_block_overrides(),
            },
        );
        id
    }

    pub fn list_state_snapshots(&self) -> BTreeMap<U256, (u64, B256)> {
        self.active_state_snapshots
            .lock()
            .iter()
            .map(|(&id, snapshot)| (id, (snapshot.block_number, snapshot.block_hash)))
            .collect()
    }

    /// Returns the EVM-visible number for a locally mined RPC block.
    fn evm_block_number(&self, rpc_number: u64) -> U256 {
        self.get_fork()
            .map_or_else(|| U256::from(rpc_number), |fork| fork.evm_block_number(rpc_number))
    }

    /// Restores the execution number separately from the RPC header number.
    fn block_env_from_header(&self, header: &impl BlockHeader) -> BlockEnv {
        let mut block = block_env_from_header(header);
        block.number = self.evm_block_number(header.number());
        if let Some(excess_blob_gas) = header.excess_blob_gas() {
            block.set_blob_excess_gas_and_price(
                excess_blob_gas,
                self.blob_params().update_fraction as u64,
            );
        }
        block
    }

    /// Returns the base fee and blob excess gas that the child of `parent` inherits.
    ///
    /// Derives the zero-base-fee sentinel from `parent` rather than the live fee state. A local
    /// genesis header may not record the fees seeded for its child, so those come from the mined
    /// child, or from the live fee state when no child has been mined yet.
    pub(crate) fn next_block_fees_after(
        &self,
        parent: &impl BlockHeader,
    ) -> (u64, BlobExcessGasAndPrice) {
        let blob_params = self.fees.blob_params();
        let update_fraction = blob_params.update_fraction as u64;
        let is_fork_root =
            self.get_fork().is_some_and(|fork| fork.block_number() == parent.number());
        if is_fork_root || (!self.is_fork() && parent.number() == self.genesis_number()) {
            let Some(child) = self.get_block(parent.number() + 1) else {
                let fees = self.fees.snapshot();
                return (fees.base_fee, fees.blob_excess_gas_and_price);
            };
            return (
                child.header.base_fee_per_gas().unwrap_or_default(),
                BlobExcessGasAndPrice::new(
                    child.header.excess_blob_gas().unwrap_or_default(),
                    update_fraction,
                ),
            );
        }
        let base_fee = parent.base_fee_per_gas().unwrap_or_default();
        let excess_blob_gas = parent.excess_blob_gas().unwrap_or_default();
        let next_base_fee = if base_fee == 0 {
            0
        } else {
            self.fees.calculate_next_block_base_fee_from_header(parent)
        };
        let next_excess_blob_gas = self.networks.next_block_blob_excess_gas(
            blob_params,
            excess_blob_gas,
            parent.blob_gas_used().unwrap_or_default(),
            base_fee,
        );
        (next_base_fee, BlobExcessGasAndPrice::new(next_excess_blob_gas, update_fraction))
    }

    /// Returns the environment for the next block
    fn next_evm_env(&self, db: &dyn DatabaseRef) -> Result<EvmEnv, DatabaseError> {
        let mut evm_env = self.evm_env.read().clone();
        // increase block number for this block
        evm_env.block_env.number = evm_env.block_env.number.saturating_add(U256::ONE);
        evm_env.block_env.basefee = self.base_fee();
        evm_env.block_env.blob_excess_gas_and_price = self.excess_blob_gas_and_price();
        let pending =
            self.prepare_block_timestamp(db, evm_env.block_env.timestamp.saturating_to(), false)?;
        evm_env.block_env.timestamp = U256::from(pending.timestamp);
        Ok(evm_env)
    }

    /// Returns the environment for replaying transactions from a historical block.
    fn tx_replay_evm_env(&self, block: &Block) -> (EvmEnv, FoundryHardfork) {
        let mut evm_env = self.evm_env.read().clone();
        evm_env.block_env = self.block_env_from_header(&block.header);
        let hardfork = self.hardfork();
        #[cfg(feature = "monad")]
        let hardfork = if self.is_monad() {
            let block_hash = block.header.hash_slow();
            let fallback = crate::eth::backend::db::MonadBlockReplayProfile {
                execution_chain_id: evm_env.cfg_env.chain_id,
                hardfork: self.monad_hardfork(),
            };
            let profile = self
                .blockchain
                .storage
                .read()
                .monad_block_replay_profiles
                .get(&block_hash)
                .copied()
                .unwrap_or(fallback);
            evm_env.cfg_env.chain_id = profile.execution_chain_id;
            let hardfork = FoundryHardfork::from(profile.hardfork);
            evm_env.cfg_env.spec = hardfork.into();
            hardfork
        } else {
            hardfork
        };
        apply_chain_specific_tx_replay_env_changes_for_chain(
            &mut evm_env,
            self.protocol_chain_id(),
        );
        (evm_env, hardfork)
    }

    /// Creates the database and environment for replaying a locally mined block.
    ///
    /// An empty block execution applies protocol-level pre-execution changes, such as the
    /// EIP-2935 parent hash system call, through the same network-specific executor used while
    /// mining.
    fn prepare_block_replay<'a>(
        &self,
        block: &Block,
        parent_state: &'a StateDb,
    ) -> Result<(CacheDB<&'a dyn DatabaseRef>, EvmEnv, FoundryHardfork), BlockchainError> {
        self.prepare_block_replay_with_db(block, parent_state)
    }

    /// Creates an overlay and applies block-start transitions for a locally mined block replay.
    fn prepare_block_replay_with_db<'a>(
        &self,
        block: &Block,
        db: &'a dyn DatabaseRef,
    ) -> Result<(CacheDB<&'a dyn DatabaseRef>, EvmEnv, FoundryHardfork), BlockchainError> {
        let (evm_env, hardfork) = self.tx_replay_evm_env(block);
        let mut cache_db = CacheDB::new(db);
        let inspector_tx_config = self.inspector_tx_config();
        let gas_config = self.pool_tx_gas_config(&evm_env);

        let executed = self.execute_with_block_executor(
            &cache_db,
            &evm_env,
            block.header.parent_hash,
            hardfork,
            block.header.parent_beacon_block_root,
            BlockExecutionKind::TransactionPrefix,
            &[],
            &gas_config,
            &inspector_tx_config,
            &|_, _| Ok(()),
        )?;
        cache_db.commit(&executed.state);

        Ok((cache_db, evm_env, hardfork))
    }

    /// Replays the stored transaction prefix `[0, end)` into an existing block overlay.
    fn replay_mined_transaction_prefix(
        &self,
        cache_db: &mut CacheDB<&dyn DatabaseRef>,
        evm_env: &EvmEnv,
        hardfork: FoundryHardfork,
        block: &Block,
        end: usize,
    ) -> Result<(), BlockchainError> {
        let monad_context = self.active_monad_context_for_mined_block(block)?;
        for (index, transaction) in block.body.transactions[..end].iter().enumerate() {
            let pending = self.pending_mined_transaction(transaction.clone())?;
            let mut inspector = AnvilInspector::default();
            let transaction_context = monad_execution_context_at(monad_context.as_ref(), index);
            let executed = self.replay_envelope_with_inspector_ref_and_context(
                &*cache_db,
                evm_env,
                &mut inspector,
                &pending,
                EnvelopeExecution::replay(transaction_context, hardfork),
            )?;
            commit_pending(cache_db, &executed.state);
        }
        Ok(())
    }

    /// Builds [`AnvilInspector`] with the configured options.
    fn build_inspector(&self) -> AnvilInspector {
        let mut inspector = AnvilInspector::default();

        if self.print_logs {
            inspector = inspector.with_log_collector();
        }
        if self.print_traces {
            inspector = inspector.with_trace_printer();
        }

        inspector
    }

    /// Builds an inspector configured for block mining (tracing always enabled).
    fn build_mining_inspector(&self) -> AnvilInspector {
        let mut inspector = AnvilInspector::default().with_tracing();
        if self.print_logs {
            inspector = inspector.with_log_collector();
        }
        if self.print_traces {
            inspector = inspector.with_trace_printer();
        }
        inspector
    }

    /// Returns a new block event stream that yields Notifications when a new block was added or
    /// when logs were removed from the canonical chain due to a reorg
    pub fn new_block_notifications(&self) -> ChainNotifications {
        let (tx, rx) = unbounded();
        self.new_block_listeners.lock().push(tx);
        trace!(target: "backed", "added new block listener");
        rx
    }

    /// Returns the number of new-block listeners. Closed listeners are pruned lazily on the next
    /// new block notification.
    pub fn new_block_listeners_count(&self) -> usize {
        self.new_block_listeners.lock().len()
    }

    /// Notifies all `new_block_listeners` about the new block
    fn notify_on_new_block(&self, header: Header, hash: B256) {
        // cleanup closed notification streams first, if the channel is closed we can remove the
        // sender half for the set
        self.new_block_listeners.lock().retain(|tx| !tx.is_closed());

        let notification =
            ChainNotification::Block(NewBlockNotification { hash, header: Arc::new(header) });

        self.new_block_listeners
            .lock()
            .retain(|tx| tx.unbounded_send(notification.clone()).is_ok());
    }

    /// Notifies all `new_block_listeners` about the logs that were removed from the canonical
    /// chain due to a reorg.
    fn notify_on_removed_logs(&self, logs: Vec<Log>) {
        // cleanup closed notification streams first, if the channel is closed we can remove the
        // sender half for the set
        self.new_block_listeners.lock().retain(|tx| !tx.is_closed());

        let notification = ChainNotification::RemovedLogs(Arc::new(logs));

        self.new_block_listeners
            .lock()
            .retain(|tx| tx.unbounded_send(notification.clone()).is_ok());
    }

    /// Returns the block number for the given block id
    pub fn convert_block_number(&self, block: Option<BlockNumber>) -> u64 {
        self.convert_block_number_at(block, self.best_number())
    }

    /// Returns the block number for the given block id, resolving head-relative tags against
    /// `current`.
    ///
    /// Callers that compare against or resolve multiple tags from the head should read it once and
    /// pass the same snapshot, since mining can advance the head between separate reads.
    pub(crate) fn convert_block_number_at(&self, block: Option<BlockNumber>, current: u64) -> u64 {
        // The chain starts at the configured genesis number, so `earliest` and the epoch-based
        // tags never resolve below it.
        let genesis = self.genesis_number();
        match block.unwrap_or(BlockNumber::Latest) {
            BlockNumber::Latest | BlockNumber::Pending => current,
            BlockNumber::Earliest => genesis,
            BlockNumber::Number(num) => num,
            BlockNumber::Safe => current.saturating_sub(self.slots_in_an_epoch).max(genesis),
            BlockNumber::Finalized => {
                current.saturating_sub(self.slots_in_an_epoch * 2).max(genesis)
            }
        }
    }

    /// Returns the canonical hash for the given block number.
    pub(crate) fn block_hash_by_number(&self, number: u64) -> Option<B256> {
        self.blockchain.hash(BlockId::number(number), self.slots_in_an_epoch)
    }

    /// Returns the block and its hash for the given id
    pub(crate) fn get_block_with_hash(&self, id: impl Into<BlockId>) -> Option<(Block, B256)> {
        let hash = self.blockchain.hash(id.into(), self.slots_in_an_epoch)?;
        let block = self.get_block_by_hash(hash)?;
        Some((block, hash))
    }

    pub fn get_block(&self, id: impl Into<BlockId>) -> Option<Block> {
        self.get_block_with_hash(id).map(|(block, _)| block)
    }

    pub fn get_block_by_hash(&self, hash: B256) -> Option<Block> {
        self.blockchain.get_block_by_hash(&hash)
    }

    /// Returns the base fees for the block after a fee history range.
    ///
    /// Mining publishes a new canonical block before advancing the fee manager. Holding the mining
    /// lock makes choosing between an existing child and the current head's pending fees atomic
    /// with that publication sequence.
    pub(crate) async fn fee_history_next_fees(&self, highest: u64) -> Option<(u128, u128)> {
        let _mining_guard = self.mining.lock().await;
        let next_number = highest.checked_add(1)?;
        if let Some(block) = self.get_block(next_number) {
            Some((
                block.header.base_fee_per_gas.unwrap_or_default() as u128,
                block.header.blob_fee(self.blob_params()).unwrap_or_default(),
            ))
        } else if highest == self.best_number() {
            Some((self.fees().base_fee() as u128, self.fees().base_fee_per_blob_gas()))
        } else {
            None
        }
    }

    /// Returns the traces for the given transaction
    pub(crate) fn mined_parity_trace_transaction(
        &self,
        hash: B256,
    ) -> Option<Vec<LocalizedTransactionTrace>> {
        self.blockchain.storage.read().transactions.get(&hash).map(|tx| tx.parity_traces())
    }

    /// Returns the traces for the given block
    pub(crate) fn mined_parity_trace_block(
        &self,
        block: u64,
    ) -> Option<Vec<LocalizedTransactionTrace>> {
        let block = self.get_block(block)?;
        let mut traces = vec![];
        let storage = self.blockchain.storage.read();
        for tx in block.body.transactions {
            if let Some(mined_tx) = storage.transactions.get(&tx.hash()) {
                traces.extend(mined_tx.parity_traces());
            }
        }
        Some(traces)
    }

    /// Returns the mined transaction for the given hash
    pub(crate) fn mined_transaction(&self, hash: B256) -> Option<MinedTransaction<N>> {
        self.blockchain.storage.read().transactions.get(&hash).cloned()
    }

    /// Overrides the given signature to impersonate the specified address during ecrecover.
    pub async fn impersonate_signature(
        &self,
        signature: Bytes,
        address: Address,
    ) -> Result<(), BlockchainError> {
        self.cheats.add_recover_override(signature, address);
        Ok(())
    }

    /// Returns code by its hash
    pub async fn debug_code_by_hash(
        &self,
        code_hash: B256,
        block_id: Option<BlockId>,
    ) -> Result<Option<Bytes>, BlockchainError> {
        if let Ok(code) = self.db.read().await.code_by_hash_ref(code_hash) {
            return Ok(Some(code.original_bytes()));
        }
        if let Some(fork) = self.get_fork() {
            return Ok(fork.debug_code_by_hash(code_hash, block_id).await?);
        }

        Ok(None)
    }

    /// Returns the value associated with a key from the database
    /// Currently only supports bytecode lookups.
    ///
    /// Based on Reth implementation: <https://github.com/paradigmxyz/reth/blob/66cfa9ed1a8c4bc2424aacf6fb2c1e67a78ee9a2/crates/rpc/rpc/src/debug.rs#L1146-L1178>
    ///
    /// Key should be: 0x63 (1-byte prefix) + 32 bytes (code_hash)
    /// Total key length must be 33 bytes.
    pub async fn debug_db_get(&self, key: String) -> Result<Option<Bytes>, BlockchainError> {
        let key_bytes = if key.starts_with("0x") {
            hex::decode(&key)
                .map_err(|_| BlockchainError::Message("Invalid hex key".to_string()))?
        } else {
            key.into_bytes()
        };

        // Validate key length: must be 33 bytes (1 byte prefix + 32 bytes code hash)
        if key_bytes.len() != 33 {
            return Err(BlockchainError::Message(format!(
                "Invalid key length: expected 33 bytes, got {}",
                key_bytes.len()
            )));
        }

        // Check for bytecode prefix (0x63 = 'c' in ASCII)
        if key_bytes[0] != 0x63 {
            return Err(BlockchainError::Message(
                "Key prefix must be 0x63 for code hash lookups".to_string(),
            ));
        }

        let code_hash = B256::from_slice(&key_bytes[1..33]);

        // Use the existing debug_code_by_hash method to retrieve the bytecode
        self.debug_code_by_hash(code_hash, None).await
    }

    fn mined_block_by_hash(&self, hash: B256) -> Option<AnyRpcBlock> {
        let block = self.blockchain.get_block_by_hash(&hash)?;
        Some(self.convert_block_with_hash(block, Some(hash)))
    }

    pub(crate) async fn mined_transactions_by_block_number(
        &self,
        number: BlockNumber,
    ) -> Option<Vec<AnyRpcTransaction>> {
        if let Some(block) = self.get_block(number) {
            return self.mined_transactions_in_block(&block);
        }
        None
    }

    /// Returns all transactions given a block
    pub(crate) fn mined_transactions_in_block(
        &self,
        block: &Block,
    ) -> Option<Vec<AnyRpcTransaction>> {
        let mut transactions = Vec::with_capacity(block.body.transactions.len());
        let base_fee = block.header.base_fee_per_gas();
        let storage = self.blockchain.storage.read();
        for hash in block.body.transactions.iter().map(|tx| tx.hash()) {
            let info = storage.transactions.get(&hash)?.info.clone();
            let tx = block.body.transactions.get(info.transaction_index as usize)?.clone();

            let tx = transaction_build(Some(hash), tx, Some(block), Some(info), base_fee);
            transactions.push(tx);
        }
        Some(transactions)
    }

    pub fn mined_block_by_number(&self, number: BlockNumber) -> Option<AnyRpcBlock> {
        let (block, hash) = self.get_block_with_hash(number)?;
        let mut block = self.convert_block_with_hash(block, Some(hash));
        block.transactions.convert_to_hashes();
        Some(block)
    }

    pub fn get_full_block(&self, id: impl Into<BlockId>) -> Option<AnyRpcBlock> {
        let (block, hash) = self.get_block_with_hash(id)?;
        let transactions = self.mined_transactions_in_block(&block)?;
        let mut block = self.convert_block_with_hash(block, Some(hash));
        block.inner.transactions = BlockTransactions::Full(transactions);
        Some(block)
    }

    /// Takes a block as it's stored internally and returns the eth api conform block format.
    pub fn convert_block(&self, block: Block) -> AnyRpcBlock {
        self.convert_block_with_hash(block, None)
    }

    /// Takes a block as it's stored internally and returns the eth api conform block format.
    /// If `known_hash` is provided, it will be used instead of computing `hash_slow()`.
    pub fn convert_block_with_hash(&self, block: Block, known_hash: Option<B256>) -> AnyRpcBlock {
        let transactions = block.body.transactions.iter().map(|tx| tx.hash()).collect();
        let block = canonical_block(block);
        let size = U256::from(block.length() as u32);
        let header = block.header;

        let hash = known_hash.unwrap_or_else(|| header.hash_slow());
        let number = header.number();
        let withdrawals_root = header.withdrawals_root();
        let consensus_context = header.as_tempo().and_then(|header| header.consensus_context);
        let tempo_fields = header
            .as_tempo()
            .map(|header| {
                (
                    header.timestamp_millis(),
                    header.general_gas_limit,
                    header.shared_gas_limit,
                    header.timestamp_millis_part,
                )
            })
            .or_else(|| {
                self.is_tempo()
                    .then(|| (header.timestamp().saturating_mul(1000), header.gas_limit(), 0, 0))
            });

        let block = AlloyBlock {
            header: AlloyHeader {
                inner: AnyHeader::from(header.into_inner()),
                hash,
                total_difficulty: Some(self.total_difficulty()),
                size: Some(size),
            },
            transactions: alloy_rpc_types::BlockTransactions::Hashes(transactions),
            uncles: vec![],
            withdrawals: withdrawals_root.map(|_| Default::default()),
        };

        let mut block = WithOtherFields::new(block);

        // If Arbitrum, apply chain specifics to converted block.
        if is_arbitrum(self.protocol_chain_id()) {
            // Set `l1BlockNumber` field.
            block.other.insert(
                "l1BlockNumber".to_string(),
                serde_json::json!(self.evm_block_number(number)),
            );
        }

        if let Some((
            timestamp_millis,
            general_gas_limit,
            shared_gas_limit,
            timestamp_millis_part,
        )) = tempo_fields
        {
            block.other.insert(
                "timestampMillis".to_string(),
                serde_json::Value::String(format!("0x{timestamp_millis:x}")),
            );
            block.other.insert(
                "mainBlockGeneralGasLimit".to_string(),
                serde_json::Value::String(format!("0x{general_gas_limit:x}")),
            );
            block.other.insert(
                "sharedGasLimit".to_string(),
                serde_json::Value::String(format!("0x{shared_gas_limit:x}")),
            );
            block.other.insert(
                "timestampMillisPart".to_string(),
                serde_json::Value::String(format!("0x{timestamp_millis_part:x}")),
            );
        }

        if let Some(consensus_context) = consensus_context {
            block.other.insert(
                "consensusContext".to_string(),
                serde_json::to_value(consensus_context)
                    .expect("Tempo consensus context serializes"),
            );
        }

        AnyRpcBlock::from(block)
    }

    pub async fn block_by_hash(&self, hash: B256) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.mined_block_by_hash(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.block_by_hash(hash).await?);
        }

        Ok(None)
    }

    pub async fn block_by_hash_full(
        &self,
        hash: B256,
    ) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by hash {:?}", hash);
        if let tx @ Some(_) = self.get_full_block(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.block_by_hash_full(hash).await?);
        }

        Ok(None)
    }

    pub async fn block_by_number(
        &self,
        number: BlockNumber,
    ) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by number {:?}", number);
        if let tx @ Some(_) = self.mined_block_by_number(number) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                return Ok(fork.block_by_number(number).await?);
            }
        }

        Ok(None)
    }

    pub async fn block_by_number_full(
        &self,
        number: BlockNumber,
    ) -> Result<Option<AnyRpcBlock>, BlockchainError> {
        trace!(target: "backend", "get block by number {:?}", number);
        if let tx @ Some(_) = self.get_full_block(number) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                return Ok(fork.block_by_number_full(number).await?);
            }
        }

        Ok(None)
    }

    /// Converts the `BlockNumber` into a numeric value
    ///
    /// # Errors
    ///
    /// returns an error if the requested number is larger than the current height
    pub async fn ensure_block_number<T: Into<BlockId>>(
        &self,
        block_id: Option<T>,
    ) -> Result<u64, BlockchainError> {
        let current = self.best_number();
        let requested = match block_id.map(Into::into).unwrap_or(BlockId::latest()) {
            BlockId::Hash(hash) => {
                self.block_by_hash(hash.block_hash)
                    .await?
                    .ok_or(BlockchainError::BlockNotFound)?
                    .header
                    .number
            }
            BlockId::Number(num) => self.convert_block_number_at(Some(num), current),
        };

        if requested > current {
            Err(BlockchainError::BlockOutOfRange(current, requested))
        } else {
            Ok(requested)
        }
    }

    /// Returns the number of the block a trace method executes against.
    ///
    /// Unlike [`Self::ensure_block_number`], this reports every block that is not part of this
    /// chain as [`BlockchainError::BlockNotFound`]: a number past the head or below the genesis
    /// number, and a hash of a block that is neither stored locally nor the fork block or an
    /// upstream block below it.
    pub async fn ensure_trace_block_number(
        &self,
        block_id: BlockId,
    ) -> Result<u64, BlockchainError> {
        let number = match self.ensure_block_number(Some(block_id)).await {
            Err(BlockchainError::BlockOutOfRange(_, _)) => {
                return Err(BlockchainError::BlockNotFound);
            }
            result => result?,
        };
        let known = match block_id {
            BlockId::Hash(hash) => {
                self.get_block_by_hash(hash.block_hash).is_some()
                    || self.get_fork().is_some_and(|fork| {
                        fork.predates_fork(number) || hash.block_hash == fork.block_hash()
                    })
            }
            BlockId::Number(_) => {
                self.get_block(number).is_some()
                    || self.get_fork().is_some_and(|fork| fork.predates_fork_inclusive(number))
            }
        };
        if known { Ok(number) } else { Err(BlockchainError::BlockNotFound) }
    }

    /// Returns the timestamp represented by a block request.
    #[cfg(feature = "base")]
    pub async fn block_request_timestamp(
        &self,
        block_request: &BlockRequest<FoundryTxEnvelope>,
    ) -> Result<u64, BlockchainError> {
        match block_request {
            BlockRequest::Pending(_) => Ok(self.time.current_call_timestamp()),
            BlockRequest::Number(number) => self
                .block_by_number(BlockNumber::Number(*number))
                .await?
                .map(|block| block.header.timestamp())
                .ok_or(BlockchainError::BlockNotFound),
        }
    }

    /// Returns the evm2 family executing this backend's network.
    const fn evm_family(&self) -> EvmFamily {
        EvmFamily::from_networks(&self.networks)
    }

    /// Returns the hardfork executing a block at `timestamp` when `hardfork` is configured.
    #[cfg_attr(not(feature = "base"), allow(unused_variables))]
    fn execution_hardfork_at(&self, hardfork: FoundryHardfork, timestamp: u64) -> FoundryHardfork {
        #[cfg(feature = "base")]
        if self.is_base() {
            let configured = match hardfork {
                FoundryHardfork::Base(upgrade) => upgrade,
                _ => BaseUpgrade::Azul,
            };
            let upgrade = if self.is_fork() {
                BaseUpgrade::from_chain_and_timestamp(self.chain_id().to(), timestamp)
                    .unwrap_or(configured)
            } else {
                configured
            };
            return FoundryHardfork::Base(upgrade);
        }
        hardfork
    }

    /// Returns the options configuring an EVM for `evm_env` at the active hardfork.
    fn evm_options(&self, evm_env: &EvmEnv) -> EvmOptions {
        self.evm_options_at(evm_env, self.hardfork())
    }

    /// Returns the options configuring an EVM for `evm_env` at `hardfork`.
    fn evm_options_at(&self, evm_env: &EvmEnv, hardfork: FoundryHardfork) -> EvmOptions {
        let timestamp = evm_env.block_env.timestamp.saturating_to();
        let hardfork = self.execution_hardfork_at(hardfork, timestamp);
        #[cfg(feature = "base")]
        let base_activation_admin = match hardfork {
            FoundryHardfork::Base(upgrade) => self.base_activation_admin().or_else(|| {
                ChainConfig::activation_admin_address_for_upgrade_by_chain_id(
                    evm_env.cfg_env.chain_id,
                    upgrade,
                )
            }),
            _ => None,
        };
        #[cfg(not(feature = "base"))]
        let base_activation_admin = None;
        EvmOptions {
            hardfork,
            celo_transfer: self.networks.is_celo(),
            precompile_factory: self.precompile_factory.clone(),
            cheats: Some(self.cheats.clone()),
            arbitrum_block_number: self.arbitrum_block_number(evm_env),
            bsc_p256: bsc_p256(self.protocol_chain_id(), timestamp),
            precompile_moves: Vec::new(),
            fork_source_chain_id: self.get_fork().map(|fork| fork.chain_id()),
            base_activation_admin,
        }
    }

    /// Executes `tx` over `db` without committing it.
    fn execute_tx<I: AnyInspector>(
        &self,
        db: &dyn DatabaseRef,
        evm_env: &EvmEnv,
        opts: &EvmOptions,
        tx: &PreparedTx,
        context: &TxContext,
        inspector: &mut I,
    ) -> Result<Executed, BlockchainError> {
        with_evm_network!(self.evm_family(), E => {
            crate::eth::backend::evm::execute::<E, I>(db, evm_env, opts, tx, context, inspector)
        })
    }

    /// Returns the evm2 precompile addresses active in `evm_env`.
    fn precompile_addresses(&self, evm_env: &EvmEnv) -> Result<Vec<Address>, BlockchainError> {
        let opts = self.evm_options(evm_env);
        let db = crate::eth::backend::db::EmptyDb;
        with_evm_network!(self.evm_family(), E => {
            let evm = build_evm::<E>(&db, evm_env, &opts)?;
            Ok(evm.precompiles().addresses())
        })
    }

    fn simulation_precompile_overrides(
        &self,
        state_overrides: Option<&StateOverride>,
        evm_env: &EvmEnv,
    ) -> Result<SimulationPrecompileOverrides, BlockchainError> {
        let mut moves = state_overrides
            .into_iter()
            .flatten()
            .filter_map(|(source, account)| {
                account.move_precompile_to.map(|destination| (*source, destination))
            })
            .collect::<Vec<_>>();
        moves.sort_unstable();
        if moves.is_empty() {
            return Ok(SimulationPrecompileOverrides::default());
        }
        if self.evm_family() != EvmFamily::Eth {
            return Err(simulate_rpc_error(
                -32000,
                "precompile moves are not supported on this network",
            ));
        }

        let precompile_addresses =
            self.precompile_addresses(evm_env)?.into_iter().collect::<HashSet<_>>();

        // Validate every source first so invalid-source errors take precedence over the more
        // specific move errors below.
        for (source, _) in &moves {
            if !precompile_addresses.contains(source) {
                return Err(simulate_rpc_error(
                    -32000,
                    format!("account {source} is not a precompile"),
                ));
            }
        }
        for (source, destination) in &moves {
            if source == destination {
                return Err(simulate_rpc_error(
                    -38022,
                    format!("cannot move precompile {source} to itself"),
                ));
            }
        }
        let mut destinations = Vec::with_capacity(moves.len());
        for (_, destination) in &moves {
            if destinations.contains(destination) {
                return Err(simulate_rpc_error(
                    -38023,
                    format!("multiple precompiles moved to {destination}"),
                ));
            }
            destinations.push(*destination);
        }

        Ok(SimulationPrecompileOverrides { moves })
    }

    /// Returns the network context of executing `tx` as the next transaction of `context`.
    ///
    /// `protocol_replay` marks `tx` as trusted mined history, whose protocol transactions replay.
    #[cfg_attr(not(feature = "monad"), allow(unused_variables))]
    fn tx_context(
        &self,
        context: Option<MonadExecutionContext<'_>>,
        tx: &PreparedTx,
        protocol_replay: bool,
    ) -> TxContext {
        #[cfg(feature = "monad")]
        if self.is_monad() {
            return TxContext {
                monad: monad::resolve_execution_context(context, tx, self.cheats()),
                monad_protocol_replay: protocol_replay,
            };
        }
        TxContext::default()
    }

    /// Executes a signed or impersonated envelope with optional Monad block context.
    fn transact_envelope_with_inspector_ref_and_context<I: AnyInspector>(
        &self,
        db: &dyn DatabaseRef,
        evm_env: &EvmEnv,
        inspector: &mut I,
        pending: &PendingTransaction<FoundryTxEnvelope>,
        monad_context: Option<MonadExecutionContext<'_>>,
    ) -> Result<Executed, BlockchainError> {
        self.replay_envelope_with_inspector_ref_and_context(
            db,
            evm_env,
            inspector,
            pending,
            EnvelopeExecution::transaction(monad_context, self.hardfork()),
        )
    }

    /// Replays a mined envelope through the active network's canonical entry point.
    fn replay_envelope_with_inspector_ref_and_context<I: AnyInspector>(
        &self,
        db: &dyn DatabaseRef,
        evm_env: &EvmEnv,
        inspector: &mut I,
        pending: &PendingTransaction<FoundryTxEnvelope>,
        execution: EnvelopeExecution<'_>,
    ) -> Result<Executed, BlockchainError> {
        let tx = PreparedTx::signed(pending.transaction.as_ref().clone(), *pending.sender());
        let opts = self.evm_options_at(evm_env, execution.hardfork);
        let context = self.tx_context(execution.monad_context, &tx, execution.protocol_replay);
        self.execute_tx(db, evm_env, &opts, &tx, &context, inspector)
    }

    /// Builds a block executor of family `N` over `db`.
    #[allow(clippy::too_many_arguments)]
    fn block_executor<'a, E: AnvilEvmNetwork>(
        &self,
        db: &'a dyn DatabaseRef,
        evm_env: &EvmEnv,
        opts: &EvmOptions,
        parent_hash: B256,
        transitions: Option<EthereumBlockTransitions>,
        inspector: AnvilInspector,
        max_blob_gas_per_block: u64,
    ) -> Result<AnvilBlockExecutor<'a, E>, BlockchainError> {
        let evm = build_evm::<E>(db, evm_env, opts)?;
        let is_cancun = evm_env.cfg_env.spec >= SpecId::CANCUN;
        #[cfg_attr(not(any(feature = "base", feature = "optimism")), allow(unused_mut))]
        let mut executor =
            AnvilBlockExecutor::new(evm, inspector, parent_hash, is_cancun, transitions)
                .with_max_blob_gas_per_block(max_blob_gas_per_block)
                .with_cheats(self.cheats.clone());
        #[cfg(any(feature = "base", feature = "optimism"))]
        {
            #[cfg(feature = "base")]
            let is_base = matches!(opts.hardfork, FoundryHardfork::Base(_));
            #[cfg(not(feature = "base"))]
            let is_base = false;
            if self.is_optimism() || is_base {
                executor = executor.with_deposit_hardfork(opts.hardfork);
            }
        }
        if matches!(
            opts.hardfork,
            FoundryHardfork::Ethereum(hardfork) if hardfork >= EthereumHardfork::Amsterdam
        ) {
            executor = executor.with_block_access_list();
        }
        Ok(executor)
    }

    /// Creates a block executor, runs pre-execution changes and executes pool transactions.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn execute_with_block_executor(
        &self,
        db: &dyn DatabaseRef,
        evm_env: &EvmEnv,
        parent_hash: B256,
        hardfork: FoundryHardfork,
        parent_beacon_block_root: Option<B256>,
        execution_kind: BlockExecutionKind,
        pool_transactions: &[Arc<PoolTransaction<FoundryTxEnvelope>>],
        gas_config: &PoolTxGasConfig,
        inspector_tx_config: &InspectorTxConfig,
        validator: &dyn Fn(
            &PoolTransaction<FoundryTxEnvelope>,
            &AccountInfo,
        ) -> Result<(), InvalidTransactionError>,
    ) -> Result<ExecutedBlock, BlockchainError> {
        let transitions =
            self.ethereum_block_transitions(hardfork, parent_beacon_block_root, execution_kind);
        let opts = self.evm_options_at(evm_env, hardfork);
        #[cfg(feature = "monad")]
        let mut monad_context =
            self.is_monad().then(|| self.monad_context_for_child_of(parent_hash)).transpose()?;
        with_evm_network!(self.evm_family(), E => {
            let mut executor = self.block_executor::<E>(
                db,
                evm_env,
                &opts,
                parent_hash,
                transitions,
                self.build_mining_inspector(),
                gas_config.max_blob_gas_per_block,
            )?;
            executor.apply_pre_execution_changes()?;
            #[cfg(feature = "monad")]
            let monad_context = std::cell::RefCell::new(monad_context.take());
            let pool = executor.execute_pool_transactions(
                pool_transactions,
                gas_config,
                inspector_tx_config,
                validator,
                &mut |_pool_tx| {
                    #[cfg(feature = "monad")]
                    if let Some(context) = monad_context.borrow_mut().as_mut() {
                        let tx = PreparedTx::signed(
                            _pool_tx.pending_transaction.transaction.as_ref().clone(),
                            *_pool_tx.pending_transaction.sender(),
                        );
                        return TxContext {
                            monad: monad::resolve_execution_context(
                                Some(MonadExecutionContext::Next(context)),
                                &tx,
                                self.cheats(),
                            ),
                            // Only forced historical replays are trusted protocol history; pool
                            // admission keeps rejecting protocol envelopes.
                            monad_protocol_replay: _pool_tx.is_replay,
                        };
                    }
                    TxContext::default()
                },
                &mut || {
                    #[cfg(feature = "monad")]
                    if let Some(context) = monad_context.borrow_mut().as_mut() {
                        monad::rollback_transaction(context);
                    }
                },
            );
            let (result, state, bal) = executor.finish()?;
            Ok(ExecutedBlock { pool, result, state, bal })
        })
    }

    /// Applies Ethereum block-start transitions to a disposable simulation candidate.
    fn apply_simulation_pre_execution_changes<DB: DatabaseRef>(
        &self,
        db: &mut CacheDB<DB>,
        evm_env: &EvmEnv,
        parent_hash: B256,
        transitions: EthereumBlockTransitions,
    ) -> Result<BalContext, BlockchainError> {
        let opts = self.evm_options(evm_env);
        let (state, bal) = {
            let mut executor = self.block_executor::<EthEvmNetwork>(
                &*db as &dyn DatabaseRef,
                evm_env,
                &opts,
                parent_hash,
                Some(transitions),
                AnvilInspector::default(),
                u64::MAX,
            )?;
            executor.apply_pre_execution_changes()?;
            let bal = std::mem::take(&mut executor.evm.overlay_db_mut().bal_context);
            (executor.state, bal)
        };
        db.commit(&state);
        Ok(bal)
    }

    /// Applies Ethereum post-block transitions to a disposable simulation candidate.
    fn apply_simulation_post_execution_changes<DB: DatabaseRef>(
        &self,
        db: &mut CacheDB<DB>,
        evm_env: &EvmEnv,
        transitions: EthereumBlockTransitions,
        receipts: &[FoundryReceiptEnvelope],
        bal: BalContext,
    ) -> Result<(alloy_eips::eip7685::Requests, Option<BlockAccessList>), BlockchainError> {
        let opts = self.evm_options(evm_env);
        let (result, state, bal) = {
            let mut executor = self.block_executor::<EthEvmNetwork>(
                &*db as &dyn DatabaseRef,
                evm_env,
                &opts,
                B256::ZERO,
                Some(transitions),
                AnvilInspector::default(),
                u64::MAX,
            )?;
            executor.evm.overlay_db_mut().bal_context = bal;
            executor.receipts = receipts.to_vec();
            executor.finish()?
        };
        db.commit(&state);
        Ok((result.requests, bal))
    }

    /// ## EVM settings
    ///
    /// This modifies certain EVM settings to mirror geth's `SkipAccountChecks` when transacting requests, see also: <https://github.com/ethereum/go-ethereum/blob/380688c636a654becc8f114438c2a5d93d2db032/core/state_transition.go#L145-L148>:
    ///
    ///  - `disable_eip3607` is set to `true`
    ///  - the base fee is zero and its check disabled for a zero-fee call, while a priced call must
    ///    pay at least the base fee
    ///  - the blob base fee is zero for a blob call without a blob fee cap
    ///  - `tx_gas_limit_cap` is set to `Some(u64::MAX)` indicating no gas limit cap, except under
    ///    EIP-8037
    ///  - `nonce` check is skipped
    fn build_call_env_with_base(
        &self,
        request: &TransactionRequest,
        fee_details: &FeeDetails,
        block_env: BlockEnv,
        base_evm_env: Option<&EvmEnv>,
    ) -> (EvmEnv, CallFees) {
        let FeeDetails {
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            max_fee_per_blob_gas,
        } = *fee_details;

        let mut evm_env = base_evm_env.cloned().unwrap_or_else(|| self.evm_env.read().clone());
        evm_env.block_env = block_env;
        // we want to disable this in eth_call, since this is common practice used by other node
        // impls and providers <https://github.com/foundry-rs/foundry/issues/4388>
        evm_env.cfg_env.disable_block_gas_limit = true;
        // Under EIP-8037 the cap stays, because gas above it becomes the state gas reservoir.
        if evm_env.cfg_env.spec < SpecId::AMSTERDAM {
            evm_env.cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        }

        // Disable the nonce check.
        evm_env.cfg_env.disable_nonce_check = true;

        let gas_price = gas_price.or(max_fee_per_gas).unwrap_or_else(|| {
            self.fees().raw_gas_price().saturating_add(MIN_SUGGESTED_PRIORITY_FEE)
        });
        // A zero-fee call runs with a zero base fee, as in geth's eth_call, so BASEFEE never
        // exceeds the price the call pays. A priced call keeps the base fee check, so a price
        // below the base fee is rejected as it is for a transaction.
        if gas_price == 0 {
            evm_env.block_env.basefee = 0;
        }
        // Set the check on every call, because a cloned base environment can carry a disabled
        // check. Pre-London blocks have no protocol base fee, so they skip the check too.
        evm_env.cfg_env.disable_base_fee =
            evm_env.block_env.basefee == 0 || evm_env.cfg_env.spec < SpecId::LONDON;
        let has_blob_hashes =
            request.blob_versioned_hashes.as_ref().is_some_and(|hashes| !hashes.is_empty());
        // As in geth's eth_call, a blob call without a blob fee cap runs at a zero blob base fee
        // and pays no blob fee, while other calls keep the block's blob base fee.
        let is_blob_call = has_blob_hashes || max_fee_per_blob_gas.is_some();
        let max_fee_per_blob_gas = max_fee_per_blob_gas.unwrap_or_default();
        if is_blob_call
            && max_fee_per_blob_gas == 0
            && let Some(blob) = evm_env.block_env.blob_excess_gas_and_price.as_mut()
        {
            blob.blob_gasprice = 0;
        }
        let fees = CallFees {
            gas_price,
            max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or_default(),
            max_fee_per_blob_gas,
            gas_limit: request.gas.unwrap_or(evm_env.block_env.gas_limit),
            chain_id: request.chain_id.unwrap_or(evm_env.cfg_env.chain_id),
        };
        (evm_env, fees)
    }

    fn prepare_call_env(
        &self,
        state: &dyn DatabaseRef,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<PreparedCall, BlockchainError> {
        self.prepare_call_env_from_base(state, request, fee_details, block_env, None)
    }

    fn prepare_call_env_from_base(
        &self,
        state: &dyn DatabaseRef,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_env: BlockEnv,
        base_evm_env: Option<&EvmEnv>,
    ) -> Result<PreparedCall, BlockchainError> {
        let gas_omitted = request.gas.is_none();
        let request = self.parse_transaction_request(request)?;
        let mut prepared = self.prepare_typed_call_env_with_base(
            state,
            request,
            fee_details,
            block_env,
            base_evm_env,
        )?;
        // Without a gas limit, a priced call gets at most the gas its sender can pay for, rather
        // than failing the funds check for the default limit. Tempo pays fees in tokens instead,
        // and OP deposits mint funds before paying for execution.
        let tx = &prepared.tx.tx;
        // A Base EIP-8130 simulation settles no fees and may be paid by a sponsor.
        #[cfg(feature = "base")]
        let is_eip8130 = tx.is_eip8130();
        #[cfg(not(feature = "base"))]
        let is_eip8130 = false;
        let cap_by_balance = !tx.is_tempo() && !is_deposit(tx) && !is_eip8130;
        let min_gas = if prepared.evm_env.cfg_env.spec >= SpecId::AMSTERDAM {
            EIP2780_TX_BASE_COST
        } else {
            MIN_TRANSACTION_GAS as u64
        };
        let gas_price = tx.max_fee_per_gas();
        if gas_omitted && cap_by_balance && gas_price > 0 {
            let balance =
                state.basic_ref(prepared.tx.sender)?.map(|info| info.balance).unwrap_or_default();
            let max_data_fee = U256::from(tx.blob_gas_used().unwrap_or_default())
                .saturating_mul(U256::from(tx.max_fee_per_blob_gas().unwrap_or_default()));
            let upfront = tx.value().saturating_add(max_data_fee);
            // A sender that cannot pay for the cheapest transaction keeps the default limit, so
            // the call fails the funds check.
            if let Some(allowance) =
                balance.checked_sub(upfront).map(|available| available / U256::from(gas_price))
                && allowance >= U256::from(min_gas)
            {
                let gas_limit = tx.gas_limit().min(allowance.saturating_to());
                prepared.tx.tx = with_gas_limit(prepared.tx.tx.clone(), gas_limit);
            }
        }
        Ok(prepared)
    }

    /// Builds the executed transaction of an Ethereum-shaped call request.
    fn build_call_tx(
        &self,
        request: TransactionRequest,
        fees: &CallFees,
        deposit: Option<DepositParts>,
    ) -> Result<PreparedTx, BlockchainError> {
        let sender = request.from.unwrap_or_default();
        #[cfg(not(any(feature = "base", feature = "optimism")))]
        let _ = deposit;
        #[cfg(any(feature = "base", feature = "optimism"))]
        if let Some(deposit) = deposit {
            let tx = op_alloy_consensus::TxDeposit {
                from: sender,
                source_hash: deposit.source_hash,
                to: request.to.unwrap_or_default(),
                mint: deposit.mint.unwrap_or_default(),
                value: request.value.unwrap_or_default(),
                gas_limit: fees.gas_limit,
                is_system_transaction: deposit.is_system_transaction,
                input: request.input.into_input().unwrap_or_default(),
            };
            return Ok(PreparedTx::simulated(
                FoundryTypedTx::Deposit(tx).into_impersonated(),
                sender,
            ));
        }
        #[cfg(not(any(feature = "base", feature = "optimism")))]
        let _ = deposit;
        let tx_type = request.minimal_tx_type();
        let to = request.to.unwrap_or_default();
        let nonce = request.nonce.unwrap_or_default();
        let value = request.value.unwrap_or_default();
        let access_list = request.access_list.clone().unwrap_or_default();
        let input = request.input.into_input().unwrap_or_default();
        let CallFees {
            gas_price,
            max_priority_fee_per_gas,
            max_fee_per_blob_gas,
            gas_limit,
            chain_id,
        } = *fees;
        let typed = match tx_type {
            alloy_consensus::TxType::Legacy => FoundryTypedTx::Legacy(alloy_consensus::TxLegacy {
                chain_id: Some(chain_id),
                nonce,
                gas_price,
                gas_limit,
                to,
                value,
                input,
            }),
            alloy_consensus::TxType::Eip2930 => {
                FoundryTypedTx::Eip2930(alloy_consensus::TxEip2930 {
                    chain_id,
                    nonce,
                    gas_price,
                    gas_limit,
                    to,
                    value,
                    access_list,
                    input,
                })
            }
            alloy_consensus::TxType::Eip1559 => {
                FoundryTypedTx::Eip1559(alloy_consensus::TxEip1559 {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee_per_gas: gas_price,
                    max_priority_fee_per_gas,
                    to,
                    value,
                    access_list,
                    input,
                })
            }
            alloy_consensus::TxType::Eip4844 => {
                let to = *to.to().ok_or(InvalidTransactionError::BlobCreateTransaction)?;
                FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844(alloy_consensus::TxEip4844 {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee_per_gas: gas_price,
                    max_priority_fee_per_gas,
                    to,
                    value,
                    access_list,
                    blob_versioned_hashes: request.blob_versioned_hashes.unwrap_or_default(),
                    max_fee_per_blob_gas,
                    input,
                }))
            }
            alloy_consensus::TxType::Eip7702 => {
                let to = *to.to().ok_or_else(|| {
                    InvalidTransactionError::Rejected(
                        "EIP-7702 transactions cannot create contracts".to_string(),
                    )
                })?;
                FoundryTypedTx::Eip7702(alloy_consensus::TxEip7702 {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee_per_gas: gas_price,
                    max_priority_fee_per_gas,
                    to,
                    value,
                    access_list,
                    authorization_list: request.authorization_list.unwrap_or_default(),
                    input,
                })
            }
        };
        Ok(PreparedTx::simulated(typed.into_impersonated(), sender))
    }

    /// Classifies an RPC request according to the active network.
    pub(crate) fn parse_transaction_request(
        &self,
        request: WithOtherFields<TransactionRequest>,
    ) -> Result<FoundryTransactionRequest, BlockchainError> {
        request
            .input
            .unique_input()
            .map_err(|err| BlockchainError::RpcError(RpcError::invalid_params(err.to_string())))?;
        // No transaction carries both fee styles, so reject the request rather than pick one.
        if request.gas_price.is_some()
            && (request.max_fee_per_gas.is_some() || request.max_priority_fee_per_gas.is_some())
        {
            return Err(BlockchainError::ConflictingFeeFields);
        }
        let transaction_type = request.transaction_type;
        if transaction_type == Some(foundry_primitives::CIP64_TX_TYPE)
            || request.other.get("feeCurrency").is_some_and(|value| !value.is_null())
        {
            self.ensure_cip64_active()?;
            return FoundryTransactionRequest::new(request)
                .map_err(|err| BlockchainError::InvalidTransactionRequest(err.to_string()));
        }
        #[cfg(feature = "base")]
        if self.is_base() {
            let parsed = FoundryTransactionRequest::try_from(request.clone()).map_err(
                |err: serde_json::Error| {
                    BlockchainError::InvalidTransactionRequest(err.to_string())
                },
            )?;
            if parsed.is_base() {
                return Ok(parsed);
            }
            if transaction_type == Some(DEPOSIT_TX_TYPE_ID)
                || get_deposit_tx_parts(&request.other).is_ok()
            {
                return Ok(FoundryTransactionRequest::Op(request));
            }
            return Ok(FoundryTransactionRequest::Ethereum(request.into_inner()));
        }
        #[cfg(feature = "base")]
        if transaction_type == Some(foundry_primitives::FoundryTxType::Eip8130.into())
            || matches!(
                FoundryTransactionRequest::try_from(request.clone()),
                Ok(FoundryTransactionRequest::Base(_))
            )
        {
            return Err(BlockchainError::BaseTransactionUnsupported);
        }
        if !self.is_tempo() && transaction_type != Some(TEMPO_TX_TYPE_ID) {
            #[cfg(feature = "optimism")]
            if transaction_type == Some(DEPOSIT_TX_TYPE_ID)
                || transaction_type == Some(POST_EXEC_TX_TYPE_ID)
                || get_deposit_tx_parts(&request.other).is_ok()
            {
                return Ok(FoundryTransactionRequest::Op(request));
            }
            return Ok(FoundryTransactionRequest::Ethereum(request.into_inner()));
        }

        let parsed: FoundryTransactionRequest =
            FoundryTransactionRequest::new(request).map_err(|err: serde_json::Error| {
                BlockchainError::InvalidTransactionRequest(err.to_string())
            })?;
        if parsed.is_tempo() {
            self.ensure_tempo_active()?;
        }
        if parsed.is_tempo()
            && self.is_tempo()
            && transaction_type.is_some_and(|ty| ty != TEMPO_TX_TYPE_ID)
        {
            return Err(BlockchainError::FailedToDecodeTransaction);
        }
        Ok(parsed)
    }

    fn build_tempo_request_env(
        &self,
        request: TempoTransactionRequest,
        fees: &CallFees,
    ) -> Result<(PreparedTx, AASigned), BlockchainError> {
        let caller = request.inner.from.unwrap_or_default();
        let fee_payer = request.fee_payer_signature.and_then(|_| {
            request.clone().build_aa().ok().and_then(|tx| tx.recover_fee_payer(caller).ok())
        });
        let mut request = request;
        request.inner.from = Some(caller);
        request.inner.gas = Some(fees.gas_limit);
        request.inner.nonce = Some(request.inner.nonce.unwrap_or_default());
        request.inner.chain_id = Some(fees.chain_id);
        request.inner.max_fee_per_gas = Some(fees.gas_price);
        request.inner.max_priority_fee_per_gas = Some(fees.max_priority_fee_per_gas);
        request.inner.access_list = Some(request.inner.access_list.clone().unwrap_or_default());
        request.inner.gas_price = None;
        let input = request.inner.input.clone().into_input().unwrap_or_default();
        if request.calls.is_empty() && request.inner.to.is_none() && !input.is_empty() {
            // Alloy represents an omitted top-level `to` as `None`; preserve Ethereum CREATE
            // semantics by materializing it as the Tempo call.
            request.inner.to = Some(TxKind::Create);
        }

        // Build the response representation separately so the mocked execution signature does not
        // leak into RPC output.
        let aa = request
            .clone()
            .build_aa()
            .map_err(|err| BlockchainError::InvalidTransactionRequest(err.to_string()))?;
        let response_tx = aa.clone().into_signed(TempoSignature::default());
        let key_type = request.key_type.unwrap_or(SignatureType::Secp256k1);
        let signature = mock_tempo_signature(
            key_type,
            request.key_data.clone(),
            request.key_id,
            caller,
            self.tempo_hardfork().is_t1c(),
        );
        let tx = PreparedTx {
            tx: FoundryTxEnvelope::Tempo(aa.into_signed(signature)),
            sender: caller,
            payload: TxPayload::Simulated,
            tempo: Some(TempoSimulation {
                unique_tx_identifier: TEMPO_RPC_SIMULATION_CONTEXT,
                fee_payer,
                key_id: request.key_id,
            }),
            eip8130_sender_actor_id: None,
        };
        Ok((tx, response_tx))
    }

    fn prepare_typed_call_env(
        &self,
        state: &dyn DatabaseRef,
        request: FoundryTransactionRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
    ) -> Result<PreparedCall, BlockchainError> {
        self.prepare_typed_call_env_with_base(state, request, fee_details, block_env, None)
    }

    fn prepare_typed_call_env_with_base(
        &self,
        state: &dyn DatabaseRef,
        request: FoundryTransactionRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
        base_evm_env: Option<&EvmEnv>,
    ) -> Result<PreparedCall, BlockchainError> {
        match request {
            FoundryTransactionRequest::Tempo(tempo_request) => {
                self.ensure_tempo_active()?;
                let mut tempo_request = *tempo_request;
                if tempo_request.inner.nonce.is_none() {
                    let caller = tempo_request.inner.from.unwrap_or_default();
                    tempo_request.inner.nonce = Some(tempo_nonce(
                        state,
                        caller,
                        tempo_request.nonce_key.unwrap_or_default(),
                    )?);
                }
                let (evm_env, fees) = self.build_call_env_with_base(
                    &tempo_request.inner,
                    &fee_details,
                    block_env,
                    base_evm_env,
                );
                let (tx, simulated_tempo_tx) =
                    self.build_tempo_request_env(tempo_request, &fees)?;
                Ok(PreparedCall {
                    evm_env,
                    tx,
                    simulated_tempo_tx: Some(simulated_tempo_tx),
                    #[cfg(feature = "base")]
                    simulated_envelope: None,
                })
            }
            #[cfg(feature = "base")]
            FoundryTransactionRequest::Base(request) => {
                self.ensure_base_eip8130_active_at(block_env.timestamp.saturating_to())?;
                let gas_limit = block_env.gas_limit;
                let (evm_env, _) = self.build_call_env_with_base(
                    request.as_ref(),
                    &fee_details,
                    block_env,
                    base_evm_env,
                );
                let invalid = || {
                    BlockchainError::InvalidTransactionRequest(
                        "invalid EIP-8130 simulation request: missing or conflicting sender, \
                         oversized authentication data, or invalid payer authenticator"
                            .to_string(),
                    )
                };
                // Base's RPC conversion resolves the sender and shapes representative sender and
                // payer authentication blobs, so intrinsic gas prices the authentication scheme.
                let simulation = request
                    .to_eip8130_simulation_tx(self.chain_id().to(), gas_limit)
                    .and_then(|tx| tx.eip8130)
                    .ok_or_else(invalid)?;
                let sender = request
                    .as_eip8130()
                    .and_then(|aa| aa.sender)
                    .or(request.as_ref().from)
                    .ok_or_else(invalid)?;
                let envelope = FoundryTxEnvelope::Eip8130(simulation.signed);
                let mut tx = PreparedTx::simulated(envelope.clone(), sender);
                tx.eip8130_sender_actor_id = simulation.simulation_sender_actor_id;
                Ok(PreparedCall {
                    evm_env,
                    tx,
                    simulated_tempo_tx: None,
                    simulated_envelope: Some(envelope),
                })
            }
            FoundryTransactionRequest::Ethereum(mut request)
            | FoundryTransactionRequest::Celo(foundry_primitives::Cip64TransactionRequest {
                inner: mut request,
                ..
            }) => {
                // Account-creation gas depends on the nonce: EIP-8037 checks the address a
                // creation would deploy to, and Tempo charges for nonce 0. So an omitted nonce
                // must not default to 0. A transaction nonce of `u64::MAX` is rejected even
                // without the nonce check, so such a sender keeps the default outside Tempo,
                // where falling back to 0 would incorrectly charge account-creation gas.
                if request.nonce.is_none() {
                    let nonce = tempo_nonce(state, request.from.unwrap_or_default(), U256::ZERO)?;
                    request.nonce = (self.is_tempo() || nonce != u64::MAX).then_some(nonce);
                }
                let (evm_env, fees) =
                    self.build_call_env_with_base(&request, &fee_details, block_env, base_evm_env);
                let tx = self.build_call_tx(request, &fees, None)?;
                Ok(PreparedCall {
                    evm_env,
                    tx,
                    simulated_tempo_tx: None,
                    #[cfg(feature = "base")]
                    simulated_envelope: None,
                })
            }
            #[cfg(any(feature = "base", feature = "optimism"))]
            FoundryTransactionRequest::Op(request) => {
                #[cfg(feature = "optimism")]
                if request.transaction_type == Some(POST_EXEC_TX_TYPE_ID) {
                    return Err(BlockchainError::InvalidTransactionRequest(
                        "OP post-execution transactions cannot be executed on evm2".to_string(),
                    ));
                }
                let deposit = (request.transaction_type == Some(DEPOSIT_TX_TYPE_ID))
                    .then(|| {
                        self.ensure_deposits_active().and_then(|()| {
                            get_deposit_tx_parts(&request.other).map_err(|missing| {
                                BlockchainError::InvalidTransactionRequest(format!(
                                    "missing deposit fields: {}",
                                    missing.join(", ")
                                ))
                            })
                        })
                    })
                    .transpose()?;
                let request = request.inner;
                let (evm_env, fees) =
                    self.build_call_env_with_base(&request, &fee_details, block_env, base_evm_env);
                let tx = self.build_call_tx(request, &fees, deposit)?;
                Ok(PreparedCall {
                    evm_env,
                    tx,
                    simulated_tempo_tx: None,
                    #[cfg(feature = "base")]
                    simulated_envelope: None,
                })
            }
        }
    }

    /// Executes a prepared call with `inspector`.
    fn transact_call_with_inspector_ref<I: AnyInspector>(
        &self,
        db: &dyn DatabaseRef,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx: &PreparedTx,
        monad_context: Option<MonadExecutionContext<'_>>,
    ) -> Result<Executed, BlockchainError> {
        self.transact_call_with_inspector_ref_at_hardfork(
            db,
            evm_env,
            inspector,
            tx,
            monad_context,
            self.hardfork(),
        )
    }

    /// Executes a prepared call with `inspector` at `hardfork`.
    fn transact_call_with_inspector_ref_at_hardfork<I: AnyInspector>(
        &self,
        db: &dyn DatabaseRef,
        evm_env: &EvmEnv,
        inspector: &mut I,
        tx: &PreparedTx,
        monad_context: Option<MonadExecutionContext<'_>>,
        hardfork: FoundryHardfork,
    ) -> Result<Executed, BlockchainError> {
        let opts = self.evm_options_at(evm_env, hardfork);
        let context = self.tx_context(monad_context, tx, false);
        self.execute_tx(db, evm_env, &opts, tx, &context, inspector)
    }

    pub(crate) fn call_with_state_and_context(
        &self,
        state: &dyn DatabaseRef,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_env: BlockEnv,
        mut monad_context: Option<MonadReplayContext>,
    ) -> Result<CallOutcome, BlockchainError> {
        let mut inspector = self.build_inspector();
        let PreparedCall { evm_env, tx, .. } =
            self.prepare_call_env(state, request, fee_details, block_env)?;
        let executed = self.transact_call_with_inspector_ref(
            state,
            &evm_env,
            &mut inspector,
            &tx,
            monad_context.as_mut().map(next_monad_context),
        )?;
        inspector.print_logs();
        if self.print_traces {
            inspector.into_print_traces(self.call_trace_decoder());
        }
        Ok(CallOutcome::from(executed))
    }

    pub(crate) fn call_with_state_typed_gas_limit(
        &self,
        state: &dyn DatabaseRef,
        request: FoundryTransactionRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
        options: GasEstimateCallOptions,
    ) -> Result<CallOutcome, BlockchainError> {
        let GasEstimateCallOptions { gas_limit, disable_fee_charge, monad_context } = options;
        self.call_with_state_typed_inner(
            state,
            request,
            fee_details,
            block_env,
            TypedCallOverrides {
                gas_limit: Some(gas_limit),
                disable_fee_charge,
                ..Default::default()
            },
            monad_context,
        )
    }

    pub(crate) fn call_with_state_typed_access_list(
        &self,
        state: &dyn DatabaseRef,
        request: FoundryTransactionRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
        access_list: AccessList,
        monad_context: Option<MonadReplayContext>,
    ) -> Result<CallOutcome, BlockchainError> {
        self.call_with_state_typed_inner(
            state,
            request,
            fee_details,
            block_env,
            TypedCallOverrides { access_list: Some(access_list), ..Default::default() },
            monad_context,
        )
    }

    fn call_with_state_typed_inner(
        &self,
        state: &dyn DatabaseRef,
        request: FoundryTransactionRequest,
        fee_details: FeeDetails,
        block_env: BlockEnv,
        overrides: TypedCallOverrides,
        mut monad_context: Option<MonadReplayContext>,
    ) -> Result<CallOutcome, BlockchainError> {
        // Estimation probes do not return or print traces, but still collect console logs.
        let mut inspector = AnvilInspector::default();
        if self.print_logs {
            inspector = inspector.with_log_collector();
        }
        let block_fees = (block_env.basefee, block_env.blob_excess_gas_and_price);
        let PreparedCall { mut evm_env, mut tx, .. } =
            self.prepare_typed_call_env(state, request, fee_details, block_env)?;
        evm_env.cfg_env.disable_fee_charge = overrides.disable_fee_charge;
        if let Some(gas_limit) = overrides.gas_limit {
            tx.tx = with_gas_limit(tx.tx, gas_limit);
        }
        if let Some(access_list) = overrides.access_list {
            Self::use_block_fees(&mut evm_env, &mut tx, block_fees);
            tx.tx = with_access_list(tx.tx, access_list)?;
        }
        let executed = self.transact_call_with_inspector_ref(
            state,
            &evm_env,
            &mut inspector,
            &tx,
            monad_context.as_mut().map(next_monad_context),
        )?;
        inspector.print_logs();
        Ok(CallOutcome::from(executed))
    }

    pub(crate) fn build_access_list_with_state_and_context(
        &self,
        state: &dyn DatabaseRef,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_env: BlockEnv,
        mut monad_context: Option<MonadReplayContext>,
    ) -> Result<(InstrStop, Bytes, u64, AccessList), BlockchainError> {
        let block_fees = (block_env.basefee, block_env.blob_excess_gas_and_price);
        let initial = request.access_list.clone().unwrap_or_default();
        let PreparedCall { mut evm_env, mut tx, .. } =
            self.prepare_call_env(state, request, fee_details, block_env)?;
        Self::use_block_fees(&mut evm_env, &mut tx, block_fees);
        let mut inspector = AccessListInspector::new(initial);
        let executed = self.transact_call_with_inspector_ref(
            state,
            &evm_env,
            &mut inspector,
            &tx,
            monad_context.as_mut().map(next_monad_context),
        )?;
        let access_list = inspector.into_access_list();
        #[cfg(feature = "monad")]
        let access_list = if self.is_monad() {
            monad::normalize_access_list(access_list, self.monad_hardfork())
        } else {
            access_list
        };
        let result = executed.result;
        Ok((result.stop, result.output.clone(), result.tx_gas_used(), access_list))
    }

    /// Restores the block's base and blob base fees that a zero-fee call env clears. Like geth,
    /// access lists are built at the block's fees.
    fn use_block_fees(
        evm_env: &mut EvmEnv,
        tx: &mut PreparedTx,
        (basefee, blob_excess_gas_and_price): (u64, Option<BlobExcessGasAndPrice>),
    ) {
        evm_env.block_env.basefee = basefee;
        evm_env.block_env.blob_excess_gas_and_price = blob_excess_gas_and_price;
        if let FoundryTxEnvelope::Eip4844(signed) = &tx.tx
            && signed.tx().tx().max_fee_per_blob_gas == 0
        {
            let blob_gasprice = evm_env.block_env.blob_gasprice().unwrap_or_default();
            let mut inner = signed.tx().tx().clone();
            inner.max_fee_per_blob_gas = blob_gasprice;
            tx.tx = FoundryTypedTx::Eip4844(TxEip4844Variant::TxEip4844(inner)).into_impersonated();
        }
    }
    fn arbitrum_block_number(&self, evm_env: &EvmEnv) -> Option<u64> {
        if !arbitrum::is_arbitrum_chain(self.protocol_chain_id()) {
            return None;
        }

        let env_block = evm_env.block_env.number;
        Some(
            self.get_fork()
                .map_or_else(|| env_block.saturating_to(), |fork| fork.rpc_block_number(env_block)),
        )
    }

    pub fn get_code_with_state(
        &self,
        state: &dyn DatabaseRef,
        address: Address,
    ) -> Result<Bytes, BlockchainError> {
        trace!(target: "backend", "get code for {:?}", address);
        let account = state.basic_ref(address)?.unwrap_or_default();
        if account.code_hash == KECCAK256_EMPTY {
            // if the code hash is `KECCAK_EMPTY`, we check no further
            return Ok(Default::default());
        }
        let code = if let Some(code) = account.code {
            code
        } else {
            state.code_by_hash_ref(account.code_hash)?
        };
        Ok(code.original_bytes())
    }

    pub fn get_balance_with_state<D>(
        &self,
        state: D,
        address: Address,
    ) -> Result<U256, BlockchainError>
    where
        D: DatabaseRef,
    {
        trace!(target: "backend", "get balance for {:?}", address);
        Ok(state.basic_ref(address)?.unwrap_or_default().balance)
    }

    pub async fn transaction_by_block_number_and_index(
        &self,
        number: BlockNumber,
        index: Index,
    ) -> Result<Option<AnyRpcTransaction>, BlockchainError> {
        if let Some(block) = self.mined_block_by_number(number) {
            return Ok(self.mined_transaction_by_block_hash_and_index(block.header.hash, index));
        }

        if let Some(fork) = self.get_fork() {
            let number = self.convert_block_number(Some(number));
            if fork.predates_fork_inclusive(number) {
                return Ok(fork
                    .transaction_by_block_number_and_index(number, index.into())
                    .await?);
            }
        }

        Ok(None)
    }

    pub async fn transaction_by_block_hash_and_index(
        &self,
        hash: B256,
        index: Index,
    ) -> Result<Option<AnyRpcTransaction>, BlockchainError> {
        if let tx @ Some(_) = self.mined_transaction_by_block_hash_and_index(hash, index) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.transaction_by_block_hash_and_index(hash, index.into()).await?);
        }

        Ok(None)
    }

    pub fn mined_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: Index,
    ) -> Option<AnyRpcTransaction> {
        let (info, block, tx) = {
            let storage = self.blockchain.storage.read();
            let block = storage.blocks.get(&block_hash).cloned()?;
            let index: usize = index.into();
            let tx = block.body.transactions.get(index)?.clone();
            let info = storage.transactions.get(&tx.hash())?.info.clone();
            (info, block, tx)
        };

        Some(transaction_build(
            Some(info.transaction_hash),
            tx,
            Some(&block),
            Some(info),
            block.header.base_fee_per_gas(),
        ))
    }

    pub async fn transaction_by_hash(
        &self,
        hash: B256,
    ) -> Result<Option<AnyRpcTransaction>, BlockchainError> {
        trace!(target: "backend", "transaction_by_hash={:?}", hash);
        if let tx @ Some(_) = self.mined_transaction_by_hash(hash) {
            return Ok(tx);
        }

        if let Some(fork) = self.get_fork() {
            return fork
                .transaction_by_hash(hash)
                .await
                .map_err(BlockchainError::AlloyForkProvider);
        }

        Ok(None)
    }

    pub fn mined_transaction_by_hash(&self, hash: B256) -> Option<AnyRpcTransaction> {
        let (info, block) = {
            let storage = self.blockchain.storage.read();
            let MinedTransaction { info, block_hash, .. } =
                storage.transactions.get(&hash)?.clone();
            let block = storage.blocks.get(&block_hash).cloned()?;
            (info, block)
        };
        let tx = block.body.transactions.get(info.transaction_index as usize)?.clone();

        Some(transaction_build(
            Some(info.transaction_hash),
            tx,
            Some(&block),
            Some(info),
            block.header.base_fee_per_gas(),
        ))
    }

    /// Returns the traces for the given transaction
    pub async fn trace_transaction(
        &self,
        hash: B256,
    ) -> Result<Option<Vec<LocalizedTransactionTrace>>, BlockchainError> {
        if let Some(traces) = self.mined_parity_trace_transaction(hash) {
            return Ok(Some(traces));
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.trace_transaction(hash).await?);
        }

        Ok(None)
    }

    /// Returns the transaction trace at the given trace address, where `[]` is the root call.
    pub async fn trace_get(
        &self,
        hash: B256,
        indices: Vec<Index>,
    ) -> Result<Option<LocalizedTransactionTrace>, BlockchainError> {
        if let Some(traces) = self.mined_parity_trace_transaction(hash) {
            let trace_address = indices.iter().copied().map(usize::from).collect::<Vec<_>>();
            return Ok(traces.into_iter().find(|trace| trace.trace.trace_address == trace_address));
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.trace_get(hash, indices).await?);
        }

        Ok(None)
    }

    /// Returns the traces for the given block
    pub async fn trace_block(
        &self,
        block: BlockNumber,
    ) -> Result<Vec<LocalizedTransactionTrace>, BlockchainError> {
        ensure_mined_trace_block(block)?;
        let number = self.convert_block_number(Some(block));
        if let Some(traces) = self.mined_parity_trace_block(number) {
            return Ok(traces);
        }

        if let Some(fork) = self.get_fork()
            && fork.predates_fork_inclusive(number)
        {
            return Ok(fork.trace_block(number).await?);
        }

        Err(BlockchainError::BlockNotFound)
    }

    /// Executes a transaction call and returns requested parity trace results.
    pub async fn trace_call(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        trace_types: HashSet<TraceType>,
        block_request: BlockRequest<FoundryTxEnvelope>,
        block_id: BlockId,
    ) -> Result<TraceResults, BlockchainError>
    where
        Self: TransactionValidator<FoundryTxEnvelope>,
        N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
    {
        if let BlockRequest::Number(number) = &block_request
            && let Some(fork) = self.get_fork()
            && fork.predates_fork(*number)
        {
            // Delegate the resolved block number so tags (`latest`/`pending`/`safe`/
            // `finalized`) are resolved against the fork's head instead of drifting with
            // the upstream chain. Hashes are forwarded unchanged.
            let resolved = match block_id {
                BlockId::Hash(_) => block_id,
                _ => BlockId::number(*number),
            };
            return Ok(fork.trace_call(request, trace_types, resolved).await?);
        }

        self.with_database_at_and_context(Some(block_request), |state, block, mut monad_context| {
            let mut inspector =
                TracingInspector::new(TracingInspectorConfig::from_parity_config(&trace_types));
            let PreparedCall { evm_env, tx, .. } =
                self.prepare_call_env(&*state, request, fee_details, block)?;
            let executed = self.transact_call_with_inspector_ref(
                &*state,
                &evm_env,
                &mut inspector,
                &tx,
                monad_context.as_mut().map(next_monad_context),
            )?;

            parity_trace_results(inspector, &executed, &trace_types, &*state, evm_env.cfg_env.spec)
        })
        .await
    }

    /// Replays all transactions in a block and returns the requested traces for each transaction.
    pub async fn trace_replay_block_transactions(
        &self,
        block: BlockNumber,
        trace_types: HashSet<TraceType>,
    ) -> Result<Vec<TraceResultsWithTransactionHash>, BlockchainError> {
        ensure_mined_trace_block(block)?;
        let block_number = self.convert_block_number(Some(block));

        // Try mined blocks first
        if let Some(results) =
            self.mined_parity_trace_replay_block_transactions(block_number, &trace_types)?
        {
            return Ok(results);
        }

        // Fallback to fork if block predates fork
        if let Some(fork) = self.get_fork()
            && fork.predates_fork_inclusive(block_number)
        {
            return Ok(fork.trace_replay_block_transactions(block_number, trace_types).await?);
        }

        Err(BlockchainError::BlockNotFound)
    }

    /// Replays a mined transaction and returns the requested traces, or `None` if the transaction
    /// is unknown.
    pub async fn trace_replay_transaction(
        &self,
        hash: B256,
        trace_types: HashSet<TraceType>,
    ) -> Result<Option<TraceResultsWithTransactionHash>, BlockchainError> {
        let mined = self.blockchain.storage.read().transactions.contains_key(&hash);

        // If the transaction was mined locally, replay it locally. Do not fall
        // through to the fork when the local replay fails; that would misreport
        // a local data problem as an upstream transaction lookup.
        let full_trace = if mined {
            let inspector =
                TracingInspector::new(TracingInspectorConfig::from_parity_config(&trace_types));
            self.replay_tx_with_inspector(
                hash,
                inspector,
                |executed, cache_db, inspector, _, env| {
                    parity_trace_results(
                        inspector,
                        &executed,
                        &trace_types,
                        cache_db,
                        env.cfg_env.spec,
                    )
                },
            )??
        } else if let Some(fork) = self.get_fork() {
            // Not known locally: forward to the fork if present.
            let Some(full_trace) = fork.trace_replay_transaction(hash, trace_types).await? else {
                return Ok(None);
            };
            full_trace
        } else {
            return Ok(None);
        };

        Ok(Some(TraceResultsWithTransactionHash { transaction_hash: hash, full_trace }))
    }

    /// Traces a raw transaction without committing it to the chain state or mempool.
    pub async fn trace_raw_transaction(
        &self,
        pending_transaction: PendingTransaction<FoundryTxEnvelope>,
        trace_types: HashSet<TraceType>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<TraceResults, BlockchainError>
    where
        N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
    {
        let trace_config = TracingInspectorConfig::from_parity_config(&trace_types);

        self.with_database_at_and_context(block_request, |state, block_env, mut monad_context| {
            let mut evm_env = self.evm_env.read().clone();
            evm_env.block_env = block_env;
            // A signed transaction cannot be impersonated, so reject senders with code that is not
            // an EIP-7702 delegation, as block execution would.
            evm_env.cfg_env.disable_eip3607 = false;

            let mut inspector = TracingInspector::new(trace_config);
            let executed = self.transact_envelope_with_inspector_ref_and_context(
                &*state,
                &evm_env,
                &mut inspector,
                &pending_transaction,
                monad_context.as_mut().map(next_monad_context),
            )?;

            parity_trace_results(inspector, &executed, &trace_types, &*state, evm_env.cfg_env.spec)
        })
        .await
    }

    /// Traces calls sequentially against a shared in-memory state.
    pub async fn trace_call_many(
        &self,
        calls: Vec<(WithOtherFields<TransactionRequest>, HashSet<TraceType>)>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<Vec<TraceResults>, BlockchainError>
    where
        N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
    {
        self.with_database_at_and_context(block_request, |state, block_env, mut monad_context| {
            let mut cache_db = CacheDB::new(&*state);
            let mut results = Vec::with_capacity(calls.len());
            let mut calls = calls.into_iter().peekable();

            while let Some((request, trace_types)) = calls.next() {
                let fee_details = FeeDetails::new(
                    request.gas_price,
                    request.max_fee_per_gas,
                    request.max_priority_fee_per_gas,
                    request.max_fee_per_blob_gas,
                )?
                .or_zero_fees();
                let PreparedCall { evm_env, mut tx, simulated_tempo_tx, .. } =
                    self.prepare_call_env(&cache_db, request, fee_details, block_env.clone())?;
                apply_tempo_envelope_identity(&mut tx, simulated_tempo_tx.as_ref());

                let trace_config = TracingInspectorConfig::from_parity_config(&trace_types);
                let mut inspector = TracingInspector::new(trace_config);
                let executed = self.transact_call_with_inspector_ref(
                    &cache_db,
                    &evm_env,
                    &mut inspector,
                    &tx,
                    monad_context.as_mut().map(next_monad_context),
                )?;

                let trace_result = parity_trace_results(
                    inspector,
                    &executed,
                    &trace_types,
                    &cache_db,
                    evm_env.cfg_env.spec,
                )?;
                results.push(trace_result);

                if calls.peek().is_some() {
                    commit_pending(&mut cache_db, &executed.state);
                }
            }

            Ok(results)
        })
        .await
    }

    /// Returns the trace results for all transactions in a mined block by replaying them
    fn mined_parity_trace_replay_block_transactions(
        &self,
        block_number: u64,
        trace_types: &HashSet<TraceType>,
    ) -> Result<Option<Vec<TraceResultsWithTransactionHash>>, BlockchainError> {
        let Some(block) = self.get_block(block_number) else { return Ok(None) };

        // Blocks without transactions, such as genesis, have nothing to replay.
        if block.body.transactions.is_empty() {
            return Ok(Some(Vec::new()));
        }

        // Execute this in the context of the parent state
        let parent_hash = block.header.parent_hash;
        let trace_config = TracingInspectorConfig::from_parity_config(trace_types);

        let read_guard = self.states.upgradable_read();
        if let Some(state) = read_guard.get_state(&parent_hash) {
            self.replay_block_transactions_with_inspector(&block, state, trace_config, trace_types)
                .map(Some)
        } else {
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            let Some(state) = write_guard.get_on_disk_state(&parent_hash) else {
                return Err(BlockchainError::HistoricalStateUnavailable(block.header.number()));
            };
            self.replay_block_transactions_with_inspector(&block, state, trace_config, trace_types)
                .map(Some)
        }
    }

    /// Replays all transactions in a block with the tracing inspector to generate TraceResults
    fn replay_block_transactions_with_inspector(
        &self,
        block: &Block,
        parent_state: &StateDb,
        trace_config: TracingInspectorConfig,
        trace_types: &HashSet<TraceType>,
    ) -> Result<Vec<TraceResultsWithTransactionHash>, BlockchainError> {
        let (mut cache_db, evm_env, hardfork) = self.prepare_block_replay(block, parent_state)?;
        let mut results = Vec::new();
        let monad_context = self.active_monad_context_for_mined_block(block)?;

        // Execute each transaction in the block with tracing
        for tx_envelope in &block.body.transactions {
            let tx_hash = tx_envelope.hash();

            // Create a fresh inspector for this transaction
            let mut inspector = TracingInspector::new(trace_config);

            // Prepare transaction environment and execute
            let pending_tx = self.pending_mined_transaction(tx_envelope.clone())?;
            let transaction_context =
                monad_execution_context_at(monad_context.as_ref(), results.len());
            let executed = self.replay_envelope_with_inspector_ref_and_context(
                &cache_db,
                &evm_env,
                &mut inspector,
                &pending_tx,
                EnvelopeExecution::replay(transaction_context, hardfork),
            )?;

            // Build TraceResults from the inspector and execution result
            let full_trace = parity_trace_results(
                inspector,
                &executed,
                trace_types,
                &cache_db,
                evm_env.cfg_env.spec,
            )?;

            results.push(TraceResultsWithTransactionHash { transaction_hash: tx_hash, full_trace });

            // Commit the state changes for the next transaction
            commit_pending(&mut cache_db, &executed.state);
        }

        Ok(results)
    }

    fn replay_tx_with_inspector<I, F, T>(
        &self,
        hash: B256,
        mut inspector: I,
        f: F,
    ) -> Result<T, BlockchainError>
    where
        I: AnyInspector,
        F: FnOnce(Executed, &dyn DatabaseRef, I, PreparedTx, EvmEnv) -> T,
    {
        let block = {
            let storage = self.blockchain.storage.read();
            let MinedTransaction { block_hash, .. } =
                storage.transactions.get(&hash).ok_or(BlockchainError::TransactionNotFound)?;

            storage.blocks.get(block_hash).cloned().ok_or(BlockchainError::BlockNotFound)?
        };

        let index = block
            .body
            .transactions
            .iter()
            .position(|tx| tx.hash() == hash)
            .expect("transaction not found in block");

        let trace = |parent_state: &StateDb| -> Result<T, BlockchainError> {
            let (mut cache_db, evm_env, hardfork) =
                self.prepare_block_replay_with_db(&block, parent_state)?;
            self.replay_mined_transaction_prefix(&mut cache_db, &evm_env, hardfork, &block, index)?;

            let target_tx = block.body.transactions[index].clone();
            let target_tx = self.pending_mined_transaction(target_tx)?;
            let monad_context = self.active_monad_context_for_mined_block(&block)?;
            let transaction_context = monad_execution_context_at(monad_context.as_ref(), index);
            let executed = self.replay_envelope_with_inspector_ref_and_context(
                &cache_db,
                &evm_env,
                &mut inspector,
                &target_tx,
                EnvelopeExecution::replay(transaction_context, hardfork),
            )?;
            let prepared =
                PreparedTx::signed(target_tx.transaction.as_ref().clone(), *target_tx.sender());

            Ok(f(executed, &cache_db, inspector, prepared, evm_env))
        };

        let read_guard = self.states.upgradable_read();
        if let Some(state) = read_guard.get_state(&block.header.parent_hash) {
            trace(state)
        } else {
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            let state = write_guard
                .get_on_disk_state(&block.header.parent_hash)
                .ok_or(BlockchainError::HistoricalStateUnavailable(block.header.number()))?;
            trace(state)
        }
    }

    // Returns the traces matching a given filter
    pub async fn trace_filter(
        &self,
        filter: TraceFilter,
    ) -> Result<Vec<LocalizedTransactionTrace>, BlockchainError> {
        let matcher = filter.matcher();
        let best_number = self.best_number();
        let start = filter.from_block.unwrap_or(best_number);
        let end = filter.to_block.unwrap_or(best_number);

        if start > best_number || end > best_number {
            return Err(BlockchainError::BlockNotFound);
        }
        if start > end {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(
                "invalid block range, ensure that to block is greater than from block".to_string(),
            )));
        }

        let dist = end - start;
        if dist > 300 {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(
                "block range too large, currently limited to 300".to_string(),
            )));
        }

        // Accumulate tasks for block range
        let mut trace_tasks = vec![];
        for num in start..=end {
            trace_tasks.push(self.trace_block(num.into()));
        }

        // Execute tasks and filter traces
        let traces = futures::future::try_join_all(trace_tasks).await?;
        let filtered_traces =
            traces.into_iter().flatten().filter(|trace| matcher.matches(&trace.trace));

        // Apply after and count
        let filtered_traces: Vec<_> = if let Some(after) = filter.after {
            filtered_traces.skip(after as usize).collect()
        } else {
            filtered_traces.collect()
        };

        let filtered_traces: Vec<_> = if let Some(count) = filter.count {
            filtered_traces.into_iter().take(count as usize).collect()
        } else {
            filtered_traces
        };

        Ok(filtered_traces)
    }

    pub fn get_blobs_by_block_id(
        &self,
        id: impl Into<BlockId>,
        versioned_hashes: Vec<B256>,
    ) -> Result<Option<Vec<alloy_consensus::Blob>>> {
        Ok(self.get_block(id).map(|block| {
            block
                .body
                .transactions
                .iter()
                .filter_map(|tx| tx.as_ref().sidecar())
                .flat_map(|sidecar| {
                    sidecar.sidecar.blobs().iter().zip(sidecar.sidecar.commitments().iter())
                })
                .filter(|(_, commitment)| {
                    // Filter blobs by versioned_hashes if provided
                    versioned_hashes.is_empty()
                        || versioned_hashes.contains(&kzg_to_versioned_hash(commitment.as_slice()))
                })
                .map(|(blob, _)| *blob)
                .collect()
        }))
    }

    #[allow(clippy::large_stack_frames)]
    pub fn get_blob_by_versioned_hash(&self, hash: B256) -> Result<Option<Blob>> {
        let storage = self.blockchain.storage.read();
        for block in storage.blocks.values() {
            for tx in &block.body.transactions {
                let typed_tx = tx.as_ref();
                if let Some(sidecar) = typed_tx.sidecar() {
                    for versioned_hash in sidecar.sidecar.versioned_hashes() {
                        if versioned_hash == hash
                            && let Some(index) =
                                sidecar.sidecar.commitments().iter().position(|commitment| {
                                    kzg_to_versioned_hash(commitment.as_slice()) == *hash
                                })
                            && let Some(blob) = sidecar.sidecar.blobs().get(index)
                        {
                            return Ok(Some(*blob));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Initialises the balance of the given accounts
    #[expect(clippy::too_many_arguments)]
    pub async fn with_genesis(
        db: Arc<AsyncRwLock<Box<dyn Db>>>,
        env: Arc<RwLock<EvmEnv>>,
        networks: NetworkConfigs,
        genesis: GenesisConfig,
        fees: FeeManager,
        fork: Arc<RwLock<Option<ClientFork>>>,
        enable_steps_tracing: bool,
        print_logs: bool,
        print_traces: bool,
        call_trace_decoder: Arc<CallTraceDecoder>,
        prune_state_history_config: PruneStateHistoryConfig,
        max_persisted_states: Option<usize>,
        transaction_block_keeper: Option<usize>,
        automine_block_time: Option<Duration>,
        cache_path: Option<PathBuf>,
        node_config: Arc<AsyncRwLock<NodeConfig>>,
    ) -> Result<Self> {
        let last_fork_cache_source = fork.read().as_ref().and_then(ForkCacheSource::from_fork);
        // if this is a fork then adjust the blockchain storage
        let blockchain = if let Some(fork) = fork.read().as_ref() {
            trace!(target: "backend", "using forked blockchain at {}", fork.block_number());
            Blockchain::forked(fork.block_number(), fork.block_hash(), fork.total_difficulty())
        } else {
            let header = if let Some(genesis) = genesis.genesis_init.as_ref() {
                genesis_json_header(genesis)
            } else {
                genesis_header(
                    &env.read(),
                    fees.is_eip1559().then(|| fees.base_fee()),
                    genesis.timestamp,
                    genesis.number,
                )
            };
            Blockchain::new(foundry_header(&networks, header))
        };

        // Sync EVM block.number with genesis for non-fork mode.
        // Fork mode syncs in setup_fork_db_config() instead.
        if fork.read().is_none() {
            env.write().block_env.number = U256::from(genesis.number);

            // The genesis block keeps its base fee, but the next block must already follow Tempo's
            // rules (e.g. T7 clamps the seed down to the cap). Fork mode seeds this from the fork
            // block instead.
            if fees.tempo_hardfork().is_some() {
                let env = env.read();
                let next_base_fee = fees.get_next_block_base_fee_per_gas(
                    0,
                    env.block_env.gas_limit,
                    env.block_env.basefee,
                );
                drop(env);
                fees.set_base_fee(next_base_fee);
            }
        }

        let start_timestamp = if let Some(fork) = fork.read().as_ref() {
            fork.timestamp()
        } else {
            genesis.timestamp
        };

        let new_states = || {
            let mut states = if prune_state_history_config.is_config_enabled() {
                // if prune state history is enabled, configure the state cache only for memory
                prune_state_history_config
                    .max_memory_history
                    .map(|limit| InMemoryBlockStates::new(limit, 0))
                    .unwrap_or_default()
                    .memory_only()
            } else if max_persisted_states.is_some() {
                max_persisted_states
                    .map(|limit| InMemoryBlockStates::new(DEFAULT_HISTORY_LIMIT, limit))
                    .unwrap_or_default()
            } else {
                Default::default()
            };

            if let Some(cache_path) = &cache_path {
                states = states.disk_path(cache_path.clone());
            }

            states
        };
        let states = new_states();

        let (slots_in_an_epoch, precompile_factory, disable_pool_balance_checks, hardfork) = {
            let cfg = node_config.read().await;
            (
                cfg.slots_in_an_epoch,
                cfg.precompile_factory.clone(),
                cfg.disable_pool_balance_checks,
                cfg.get_hardfork(),
            )
        };
        if networks.is_tempo() {
            let cfg = node_config.read().await;
            let mut storage = blockchain.storage.write();
            let mut tempo = TempoChain::new(
                cfg.tempo_consensus_seed(),
                cfg.tempo_epoch_length,
                storage.best_number,
            );
            let mut proof_states = new_states();
            if prune_state_history_config.is_state_history_supported() {
                // Execution history counts past states; proof history also retains the head.
                proof_states = proof_states.with_current_state();
            } else {
                proof_states.set_cache_limit(1);
            }
            tempo.proof_states = Arc::new(RwLock::new(proof_states));
            storage.tempo = Some(tempo);
        }
        let startup_cache_lease = if fork.read().is_some() {
            let db = db.read().await;
            let inner = db
                .maybe_inner()
                .map_err(|err| eyre::eyre!("fork database is missing its cache backend: {err}"))?;
            StagedForkCacheLease::for_db(inner)
        } else {
            StagedForkCacheLease::default()
        };
        let startup_fork_cache_user =
            StagedForkDbUser { db: Some(Arc::clone(&db)), cache_lease: startup_cache_lease };
        #[cfg(feature = "base")]
        let base_activation_admin = node_config.read().await.base_activation_admin;

        let backend = Self {
            db,
            blockchain,
            states: Arc::new(RwLock::new(states)),
            evm_env: env,
            networks,
            #[cfg(feature = "base")]
            base_activation_admin,
            hardfork: Arc::new(RwLock::new(hardfork)),
            fork,
            last_fork_cache_source: Arc::new(RwLock::new(last_fork_cache_source)),
            time: TimeManager::new(start_timestamp),
            cheats: Default::default(),
            new_block_listeners: Default::default(),
            fees,
            genesis,
            active_state_snapshots: Arc::new(Mutex::new(Default::default())),
            enable_steps_tracing,
            print_logs,
            print_traces,
            call_trace_decoder: Arc::new(RwLock::new(call_trace_decoder)),
            prune_state_history_config,
            transaction_block_keeper,
            node_config,
            slots_in_an_epoch,
            precompile_factory,
            mining: Arc::new(tokio::sync::Mutex::new(())),
            #[cfg(test)]
            mining_commit_hook: Default::default(),
            #[cfg(test)]
            mining_state_root_hook: Default::default(),
            #[cfg(test)]
            rollback_unwind_hook: Default::default(),
            disable_pool_balance_checks,
            startup_fork_cache_user,
        };

        #[cfg(feature = "monad")]
        let monad_fork =
            if backend.networks.is_monad() { backend.fork.read().clone() } else { None };
        #[cfg(feature = "monad")]
        if let Some(fork) = monad_fork {
            monad::cache_fork_context(&fork).await?;
        }

        if let Some(interval_block_time) = automine_block_time {
            backend.update_interval_mine_block_time(interval_block_time);
        }

        // Note: this can only fail in forking mode, in which case we can't recover
        backend.apply_genesis().await.wrap_err("failed to create genesis")?;
        Ok(backend)
    }

    /// Applies the configured genesis settings
    ///
    /// This will fund, create the genesis accounts
    async fn apply_genesis(&self) -> Result<(), DatabaseError> {
        trace!(target: "backend", "setting genesis balances");

        if self.fork.read().is_some() {
            let parent_env = self.evm_env.read().clone();
            return self
                .apply_fork_genesis(
                    Arc::clone(&self.db),
                    self.startup_fork_cache_user.cache_lease.clone(),
                    &parent_env,
                    self.best_hash(),
                    self.hardfork(),
                )
                .await;
        }

        let mut db = self.db.write().await;
        for (account, info) in self.genesis.account_infos() {
            db.insert_account(account, info);
        }

        // insert the new genesis hash to the database so it's available for the next block in
        // the evm
        db.insert_block_hash(U256::from(self.best_number()), self.best_hash());

        if let Some(transitions) =
            self.ethereum_block_transitions(self.hardfork(), None, BlockExecutionKind::Complete)
        {
            if transitions.hardfork >= EthereumHardfork::Cancun {
                db.set_code(eip4788::BEACON_ROOTS_ADDRESS, eip4788::BEACON_ROOTS_CODE.clone())?;
            }
            if transitions.hardfork >= EthereumHardfork::Prague {
                db.set_code(
                    eip2935::HISTORY_STORAGE_ADDRESS,
                    eip2935::HISTORY_STORAGE_CODE.clone(),
                )?;
                db.set_code(
                    eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
                    eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_CODE.clone(),
                )?;
                db.set_code(
                    eip7251::CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
                    eip7251::CONSOLIDATION_REQUEST_PREDEPLOY_CODE.clone(),
                )?;
            }
        }
        // apply the genesis.json alloc
        self.genesis.apply_genesis_json_alloc(&mut **db)?;
        drop(db);
        self.apply_funded_accounts(&self.db).await?;

        // Seed Base protocol accounts and one-time state transitions for standalone nodes.
        #[cfg(feature = "base")]
        if self.is_base() && !self.is_fork() {
            let upgrade = self.base_upgrade();
            Self::apply_base_genesis(&mut **self.db.write().await, upgrade)?;
        }

        // Initialize Tempo precompiles and fee tokens when in Tempo mode (not in fork mode).
        // In fork mode, precompiles are inherited from the forked origin.
        if self.networks.is_tempo() && !self.is_fork() {
            let chain_id = self.evm_env.read().cfg_env.chain_id;
            let timestamp = self.genesis.timestamp;
            let test_accounts: Vec<Address> = self.genesis.accounts.clone();
            let hardfork = self.tempo_hardfork();
            let mut db = self.db.write().await;
            crate::eth::backend::tempo::initialize_tempo_precompiles(
                &mut **db,
                &tempo_genesis_env(chain_id, timestamp, hardfork),
                &test_accounts,
                hardfork.into(),
            )
            .map_err(|e| {
                tracing::error!(target: "backend", "failed to initialize Tempo precompiles: {e}");
                DatabaseError::AnyRequest(Arc::new(eyre::eyre!("{e}")))
            })?;
            trace!(target: "backend", "initialized Tempo precompiles and fee tokens for {} accounts", test_accounts.len());
        }

        trace!(target: "backend", "set genesis balances");

        Ok(())
    }

    /// Applies genesis allocations to a fork database before it becomes live.
    async fn apply_fork_genesis(
        &self,
        db: Arc<AsyncRwLock<Box<dyn Db>>>,
        cache_lease: StagedForkCacheLease,
        parent_env: &EvmEnv,
        parent_hash: B256,
        hardfork: FoundryHardfork,
    ) -> Result<(), DatabaseError> {
        let user = StagedForkDbUser { db: Some(db), cache_lease };
        let mut genesis_accounts = JoinSet::new();
        for address in self.genesis.accounts.iter().copied() {
            let task_user = StagedForkDbUser {
                db: Some(Arc::clone(user.db())),
                cache_lease: user.cache_lease.clone(),
            };

            // The fork database can fetch independent accounts concurrently.
            genesis_accounts.spawn(async move {
                let db = task_user.db().read().await;
                let info = db.basic_ref(address)?.unwrap_or_default();
                Ok::<_, DatabaseError>((address, info))
            });
        }

        let mut account_infos = Vec::with_capacity(self.genesis.accounts.len());
        while let Some(result) = genesis_accounts.join_next().await {
            match result {
                Ok(Ok(account)) => account_infos.push(account),
                Ok(Err(err)) => {
                    genesis_accounts.shutdown().await;
                    return Err(err);
                }
                Err(err) => {
                    genesis_accounts.shutdown().await;
                    return Err(DatabaseError::AnyRequest(Arc::new(eyre::eyre!(
                        "fork genesis account task failed: {err}"
                    ))));
                }
            }
        }
        let mut db_guard = user.db().write().await;
        for (address, mut info) in account_infos {
            info.balance = self.genesis.balance;
            db_guard.insert_account(address, info);
        }
        self.genesis.apply_genesis_json_alloc(&mut **db_guard)?;
        // A fork taken before the remote chain activated Denim has no BaseTime deployment, but
        // Denim blocks mined locally still send the BaseTime deposit. Existing deployments and
        // their admin are preserved.
        #[cfg(feature = "base")]
        if matches!(hardfork, FoundryHardfork::Base(upgrade) if upgrade >= BaseUpgrade::Denim) {
            base::ensure_base_time_predeploy(&mut **db_guard, false)?;
            self.ensure_fork_accepts_system_transactions(
                &**db_guard,
                parent_env,
                parent_hash,
                hardfork,
            )?;
        }
        #[cfg(not(feature = "base"))]
        let _ = (parent_env, parent_hash, hardfork);
        drop(db_guard);
        self.apply_funded_accounts(user.db()).await
    }

    /// Applies explicit `--fund` balances while preserving account metadata inherited from a fork.
    async fn apply_funded_accounts(
        &self,
        db: &Arc<AsyncRwLock<Box<dyn Db>>>,
    ) -> Result<(), DatabaseError> {
        let funded_accounts = self.node_config.read().await.funded_accounts.clone();
        let mut accounts = Vec::with_capacity(funded_accounts.len());
        for (address, balance) in funded_accounts {
            let mut info = db.read().await.basic_ref(address)?.unwrap_or_default();
            info.balance = balance;
            accounts.push((address, info));
        }
        let mut db = db.write().await;
        for (address, info) in accounts {
            db.insert_account(address, info);
        }
        Ok(())
    }

    /// Seeds Base protocol accounts and one-time state transitions for a standalone node.
    #[cfg(feature = "base")]
    fn apply_base_genesis(db: &mut dyn Db, upgrade: BaseUpgrade) -> Result<(), DatabaseError> {
        for address in [
            Predeploys::L1_BLOCK_INFO,
            Predeploys::SEQUENCER_FEE_VAULT,
            Predeploys::BASE_FEE_VAULT,
            Predeploys::L1_FEE_VAULT,
            Predeploys::OPERATOR_FEE_VAULT,
            Predeploys::BASE_TIME,
        ] {
            if db.basic_ref(address)?.is_none() {
                db.insert_account(address, AccountInfo::default());
            }
        }

        db.set_storage_at(
            Predeploys::L1_BLOCK_INFO,
            L1BlockInfo::L1_BASE_FEE_SLOT.into(),
            U256::from(DEFAULT_BASE_L1_BASE_FEE).into(),
        )?;
        db.set_storage_at(
            Predeploys::L1_BLOCK_INFO,
            L1BlockInfo::ECOTONE_L1_BLOB_BASE_FEE_SLOT.into(),
            B256::with_last_byte(1),
        )?;
        let mut l1_fee_scalars = [0u8; 32];
        l1_fee_scalars
            [L1BlockInfo::BASE_FEE_SCALAR_OFFSET..L1BlockInfo::BASE_FEE_SCALAR_OFFSET + 4]
            .copy_from_slice(&DEFAULT_BASE_L1_FEE_SCALAR.to_be_bytes());
        db.set_storage_at(
            Predeploys::L1_BLOCK_INFO,
            L1BlockInfo::ECOTONE_L1_FEE_SCALARS_SLOT.into(),
            B256::from(l1_fee_scalars),
        )?;

        if upgrade >= BaseUpgrade::Canyon {
            base::ensure_create2_deployer(db)?;
        }
        if upgrade >= BaseUpgrade::Denim {
            base::ensure_base_time_predeploy(db, true)?;
        }
        if upgrade >= BaseUpgrade::Zenith {
            base::ensure_eip8130_system_accounts(db)?;
        }

        // Give the installed Base precompiles a sentinel byte so Solidity's `extcodesize`
        // check on high-level calls to functions without return data does not revert in the
        // caller. `ensure_eip8130_system_accounts` only covers the Zenith nonce manager.
        for address in foundry_evm_networks::BASE_CODE_SENTINEL_ADDRESSES
            .iter()
            .copied()
            .filter(|address| foundry_evm_networks::is_base_precompile_active_at(*address, upgrade))
        {
            let mut account = db.basic_ref(address)?.unwrap_or_default();
            if account.code.as_ref().is_none_or(|code| code.is_empty()) {
                let code = Bytecode::new_legacy(Bytes::from_static(
                    foundry_evm_core::constants::SYSTEM_PRECOMPILE_STUB,
                ));
                account.set_code(code);
                db.insert_account(address, account);
            }
        }

        Ok(())
    }

    /// Populates a detached in-memory database from explicit reset inputs.
    #[allow(clippy::too_many_arguments)]
    fn populate_memory_db(
        db: &mut dyn Db,
        genesis: &GenesisConfig,
        funded_accounts: &HashMap<Address, U256>,
        hardfork: FoundryHardfork,
        chain_id: u64,
        is_tempo: bool,
        tempo_hardfork: Option<TempoHardfork>,
        genesis_hash: B256,
        install_create2_deployer: bool,
    ) -> Result<(), DatabaseError> {
        for (account, info) in genesis.account_infos() {
            db.insert_account(account, info);
        }
        db.insert_block_hash(U256::from(genesis.number), genesis_hash);

        if let FoundryHardfork::Ethereum(hardfork) = hardfork {
            if hardfork >= EthereumHardfork::Cancun {
                db.set_code(eip4788::BEACON_ROOTS_ADDRESS, eip4788::BEACON_ROOTS_CODE.clone())?;
            }
            if hardfork >= EthereumHardfork::Prague {
                db.set_code(
                    eip2935::HISTORY_STORAGE_ADDRESS,
                    eip2935::HISTORY_STORAGE_CODE.clone(),
                )?;
                db.set_code(
                    eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_ADDRESS,
                    eip7002::WITHDRAWAL_REQUEST_PREDEPLOY_CODE.clone(),
                )?;
                db.set_code(
                    eip7251::CONSOLIDATION_REQUEST_PREDEPLOY_ADDRESS,
                    eip7251::CONSOLIDATION_REQUEST_PREDEPLOY_CODE.clone(),
                )?;
            }
        }

        genesis.apply_genesis_json_alloc(db)?;
        for (&address, &balance) in funded_accounts {
            let mut info = db.basic_ref(address)?.unwrap_or_default();
            info.balance = balance;
            db.insert_account(address, info);
        }

        if is_tempo {
            let hardfork = tempo_hardfork.ok_or_else(|| {
                DatabaseError::AnyRequest(Arc::new(eyre::eyre!(
                    "missing Tempo hardfork during memory reset"
                )))
            })?;
            crate::eth::backend::tempo::initialize_tempo_precompiles(
                db,
                &tempo_genesis_env(chain_id, genesis.timestamp, hardfork),
                &genesis.accounts,
                hardfork.into(),
            )
            .map_err(|err| DatabaseError::AnyRequest(Arc::new(eyre::eyre!("{err}"))))?;
        }

        if install_create2_deployer {
            db.set_code(
                DEFAULT_CREATE2_DEPLOYER,
                Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE),
            )?;
        }

        Ok(())
    }

    /// Prepares a fresh fork without mutating the live backend.
    pub(crate) async fn prepare_fork_reset(
        &self,
        forking: Forking,
        serving_instance_id: B256,
    ) -> Result<StagedForkReset, BlockchainError> {
        let previous_fork = self.get_fork();
        let previous_source = self
            .last_fork_cache_source
            .read()
            .clone()
            .or_else(|| previous_fork.as_ref().and_then(ForkCacheSource::from_fork));
        let configured_rpc_urls = self.node_config.read().await.fork_urls.clone();
        let rpc_url_was_provided = forking.json_rpc_url.is_some();
        let target_rpc_urls = if let Some(url) = forking.json_rpc_url {
            vec![url]
        } else if !configured_rpc_urls.is_empty() {
            configured_rpc_urls
        } else {
            previous_fork
                .as_ref()
                .map(|fork| fork.config.read().fork_urls.clone())
                .filter(|urls| !urls.is_empty())
                .ok_or_else(|| {
                    RpcError::invalid_params(
                        "Forking not enabled and RPC URL not provided to start forking",
                    )
                })?
        };
        let flush_old_cache = previous_fork.is_some();
        if flush_old_cache {
            // Staging opens a separate BlockchainDb from disk. Persist the live remote cache first
            // so an unchanged source and block can reuse it without copying locally modified state.
            self.db.write().await.maybe_flush_cache().map_err(BlockchainError::Internal)?;
        }

        for _ in 0..3 {
            if let Some(staged) = self
                .stage_fork_reset(
                    &target_rpc_urls,
                    forking.block_number,
                    serving_instance_id,
                    previous_source.clone(),
                    flush_old_cache,
                    rpc_url_was_provided,
                )
                .await?
            {
                return Ok(staged);
            }
        }
        Err(BlockchainError::Internal(
            "fork endpoint changed while the replacement was being staged".to_string(),
        ))
    }

    /// Builds and validates one complete fork replacement without mutating the live backend.
    async fn stage_fork_reset(
        &self,
        target_rpc_urls: &[String],
        block_number: Option<u64>,
        serving_instance_id: B256,
        previous_source: Option<ForkCacheSource>,
        flush_old_cache: bool,
        rpc_url_was_provided: bool,
    ) -> Result<Option<StagedForkReset>, BlockchainError> {
        let target_rpc_url = target_rpc_urls.first().ok_or_else(|| {
            BlockchainError::Internal("at least one fork URL is required".to_string())
        })?;
        let mut staged_config = self.node_config.read().await.clone();
        if rpc_url_was_provided {
            staged_config.fork_chain_id = None;
        }
        staged_config.warn_if_fork_headers_withheld(target_rpc_url);
        let configured_endpoint_is_anvil = staged_config.fork_endpoint_is_anvil
            && staged_config.fork_urls.contains(target_rpc_url);
        let cached_endpoint_is_anvil = previous_source.as_ref().is_some_and(|source| {
            source.rpc_url == *target_rpc_url && source.endpoint_identity.is_authoritative()
        });
        staged_config.fork_endpoint_is_anvil =
            configured_endpoint_is_anvil || cached_endpoint_is_anvil;
        staged_config.fork_urls = target_rpc_urls.to_vec();
        staged_config.fork_choice = block_number.map(|number| ForkChoice::Block(number as i128));
        let mut staged_env = self.evm_env.read().clone();
        staged_config.apply_tempo_fork_beneficiary_default(&mut staged_env);
        let staged_fees = self.fees.detached();
        let (mut staged_db, staged_client_config) = staged_config
            .setup_fork_db_config_for_reset(
                target_rpc_url.clone(),
                &mut staged_env,
                &staged_fees,
                self.networks,
            )
            .await?;
        let cache_lease = StagedForkCacheLease::for_db(staged_db.inner());
        let cache_identity_changed = previous_source.as_ref().is_some_and(|source| {
            source.authoritative_identity_changed_at_same_url(
                target_rpc_url,
                staged_client_config.endpoint_identity,
            )
        });
        if cache_identity_changed {
            staged_db.clear_into_state_snapshot();
            staged_db.insert_block_hash(
                U256::from(staged_client_config.block_number),
                staged_client_config.block_hash,
            );
        }
        if !staged_config.no_bal && !staged_config.no_fork_node_info {
            staged_client_config.prefill_cache(staged_db.inner()).await;
        }
        let mut invalidated_cache_namespaces = Vec::new();
        if cache_identity_changed && !staged_config.no_storage_caching {
            if let Some(source) = &previous_source
                && let Some(namespace) = ForkCacheNamespace::new(
                    source.endpoint_identity.source_chain_id,
                    target_rpc_url,
                )
            {
                invalidated_cache_namespaces.push(namespace);
            }
            if let Some(namespace) =
                ForkCacheNamespace::new(staged_client_config.chain_id, target_rpc_url)
                && !invalidated_cache_namespaces.contains(&namespace)
            {
                invalidated_cache_namespaces.push(namespace);
            }
        }
        let discard_old_cached_state =
            cache_identity_changed && flush_old_cache && !invalidated_cache_namespaces.is_empty();
        let staged_db: Arc<AsyncRwLock<Box<dyn Db>>> =
            Arc::new(AsyncRwLock::new(Box::new(staged_db)));
        let staged_fork = ClientFork::new(staged_client_config.clone(), Arc::clone(&staged_db));
        let attempt = async {
            let target_networks =
                staged_client_config.endpoint_identity.network_profile.unwrap_or_default();
            if !staged_config.has_explicit_network_selection()
                && !self.networks.supports_fork_source(&target_networks)
            {
                return Err(RpcError::invalid_params(format!(
                    "cannot reset Anvil across network families ({} -> {}); start a new \
                     instance with matching network configuration",
                    self.execution_profile_name(),
                    target_networks.execution_profile_name()
                ))
                .into());
            }
            if staged_client_config.endpoint_identity.instance_id == Some(serving_instance_id) {
                return Err(
                    RpcError::invalid_params("cannot reset Anvil to its own RPC endpoint").into()
                );
            }
            let fork_block = staged_fork
                .block_by_number(staged_fork.block_number())
                .await?
                .ok_or(BlockchainError::BlockNotFound)?;
            if fork_block.header.hash != staged_client_config.block_hash {
                return Ok(None);
            }
            self.apply_fork_genesis(
                Arc::clone(&staged_db),
                cache_lease.clone(),
                &staged_env,
                staged_client_config.block_hash,
                staged_client_config.hardfork.unwrap_or_else(|| staged_config.get_hardfork()),
            )
            .await?;

            #[cfg(feature = "monad")]
            if self.is_monad() {
                monad::cache_fork_context(&staged_fork).await?;
            }

            if !staged_config
                .fork_urls_match_context(
                    target_rpc_urls,
                    staged_client_config.endpoint_identity,
                    staged_client_config.block_number,
                    staged_client_config.block_hash,
                )
                .await?
            {
                return Ok(None);
            }

            Ok(Some((fork_block, staged_fork.storage.read().clone())))
        }
        .await;
        drop(staged_fork);
        let (fork_block, staged_storage) = match attempt {
            Ok(Some(staged)) => staged,
            Ok(None) => {
                drop(staged_db);
                self.rollback_staged_fork_cache(cache_lease, flush_old_cache).await?;
                return Ok(None);
            }
            Err(err) => {
                drop(staged_db);
                self.rollback_staged_fork_cache(cache_lease, flush_old_cache).await?;
                return Err(err);
            }
        };
        let staged_db = match Arc::try_unwrap(staged_db) {
            Ok(staged_db) => staged_db.into_inner(),
            Err(staged_db) => {
                drop(staged_db);
                self.rollback_staged_fork_cache(cache_lease, flush_old_cache).await?;
                return Err(BlockchainError::Internal(
                    "staged fork database still has active references".to_string(),
                ));
            }
        };

        let fork = ClientFork::new(staged_client_config, Arc::clone(&self.db));
        *fork.storage.write() = staged_storage;
        Ok(Some(StagedForkReset {
            node_config: staged_config,
            db: staged_db,
            fees: staged_fees,
            evm_env: staged_env,
            fork,
            timestamp: fork_block.header.timestamp(),
            discard_old_cached_state,
            invalidated_cache_namespaces,
            flush_old_cache,
            cache_lease,
        }))
    }

    async fn rollback_staged_fork_cache(
        &self,
        cache_lease: StagedForkCacheLease,
        restore_live_cache: bool,
    ) -> Result<(), BlockchainError> {
        let rollback_err = cache_lease.rollback().err();
        // If immediate invalidation failed, dropping the final lease retries cleanup before the
        // live cache is restored at a potentially shared path.
        drop(cache_lease);
        let restore_err =
            if restore_live_cache { self.db.read().await.maybe_flush_cache().err() } else { None };
        match (rollback_err, restore_err) {
            (None, None) => Ok(()),
            (Some(err), None) => Err(err),
            (None, Some(err)) => Err(BlockchainError::Internal(format!(
                "failed to restore the live fork cache after staged reset rollback: {err}"
            ))),
            (Some(rollback), Some(restore)) => Err(BlockchainError::Internal(format!(
                "{rollback}; restoring the live fork cache also failed: {restore}"
            ))),
        }
    }

    /// Atomically publishes a fully prepared fork replacement.
    pub(crate) async fn commit_fork_reset(
        &self,
        staged: StagedForkReset,
    ) -> Result<(), BlockchainError> {
        let fork_block_number = staged.fork.block_number();
        let fork_block_hash = staged.fork.block_hash();
        let fork_total_difficulty = staged.fork.total_difficulty();
        let fork_cache_source = ForkCacheSource::from_fork(&staged.fork);

        // Acquire asynchronous write guards before flushing and replacing the live database so no
        // old-context request can populate its cache between those operations.
        let mut node_config = self.node_config.write().await;
        let mut db = self.db.write().await;
        if staged.flush_old_cache {
            db.maybe_flush_cache().map_err(BlockchainError::Internal)?;
        }
        if let Err(err) =
            staged.invalidated_cache_namespaces.iter().try_for_each(ForkCacheNamespace::invalidate)
        {
            // Prevent the rejected staged backend from flushing partial target state, then restore
            // the still-live cache after the endpoint namespace cleanup failed.
            let StagedForkReset { db: staged_db, cache_lease, flush_old_cache, .. } = staged;
            drop(staged_db);
            let rollback_err = cache_lease.rollback().err();
            drop(cache_lease);
            let restore_err = if flush_old_cache { db.maybe_flush_cache().err() } else { None };
            let mut message = err.to_string();
            if let Some(err) = rollback_err {
                message.push_str(&format!("; staged cache rollback also failed: {err}"));
            }
            if let Some(err) = restore_err {
                message.push_str(&format!("; restoring the live fork cache also failed: {err}"));
            }
            return Err(BlockchainError::Internal(message));
        }
        if staged.discard_old_cached_state {
            db.clear_into_state_snapshot();
        }
        staged.cache_lease.disarm();
        let StagedForkReset {
            node_config: staged_node_config,
            db: staged_db,
            fees,
            evm_env,
            fork,
            timestamp,
            ..
        } = staged;
        *node_config = staged_node_config;
        *db = staged_db;
        self.fees.replace_from(&fees);
        *self.evm_env.write() = evm_env;
        *self.fork.write() = Some(fork);
        *self.last_fork_cache_source.write() = fork_cache_source;
        {
            let mut storage = self.blockchain.storage.write();
            let mut replacement = BlockchainStorage::forked(
                fork_block_number,
                fork_block_hash,
                fork_total_difficulty,
            );
            replacement.tempo = storage.tempo.take();
            if let Some(tempo) = &mut replacement.tempo {
                tempo.reanchor(fork_block_number);
            }
            *storage = replacement;
        }
        self.states.write().clear();
        self.active_state_snapshots.lock().clear();
        self.time.reset(timestamp);
        self.cheats.clear_next_block_overrides();

        trace!(target: "backend", "reset fork");
        Ok(())
    }

    /// Builds a complete in-memory replacement without mutating the live backend.
    pub(crate) async fn prepare_memory_reset(
        &self,
    ) -> Result<StagedMemoryReset<N>, BlockchainError> {
        let reset_from_fork = self.is_fork();
        let genesis_timestamp = self.genesis.timestamp;
        let genesis_number = self.genesis.number;
        let mut staged_config = self.node_config.read().await.clone();
        staged_config.fork_source_chain_id = None;
        staged_config.fork_execution_chain_id = None;
        staged_config.fork_endpoint_is_anvil = false;
        staged_config.restore_fork_overrides();
        let local_chain_id = staged_config.get_chain_id();
        staged_config.update_wallet_chain_id(local_chain_id);
        let (
            local_gas_limit,
            local_base_fee,
            local_base_fee_is_explicit,
            local_gas_price,
            local_blob_params,
            local_blob_excess_gas_and_price,
            local_beneficiary,
            install_create2_deployer,
        ) = {
            (
                staged_config.gas_limit(),
                staged_config.get_base_fee(),
                staged_config.base_fee.is_some()
                    || staged_config
                        .genesis
                        .as_ref()
                        .is_some_and(|genesis| genesis.base_fee_per_gas.is_some()),
                staged_config.get_gas_price(),
                staged_config.get_blob_params(),
                staged_config.get_blob_excess_gas_and_price(),
                staged_config.genesis.as_ref().map(|genesis| genesis.coinbase).unwrap_or_default(),
                !staged_config.disable_default_create2_deployer,
            )
        };

        let local_hardfork = staged_config.get_hardfork();
        let local_spec = SpecId::from(local_hardfork);
        let local_tempo_hardfork =
            self.networks.is_tempo().then(|| TempoHardfork::from(local_hardfork));
        let staged_fees = self.fees.detached();
        staged_fees.set_execution_rules(
            local_spec,
            self.networks.base_fee_params(genesis_timestamp),
            local_tempo_hardfork,
        );
        #[cfg(feature = "optimism")]
        if self.networks.is_optimism() {
            staged_fees.set_optimism_hardfork(local_hardfork.into());
        }
        staged_fees.set_blob_params(local_blob_params);
        staged_fees.set_blob_excess_gas_and_price(local_blob_excess_gas_and_price);

        // Explicit local configuration always wins, while Tempo configuration uses the
        // hardfork's own seed. An implicit in-memory Ethereum chain otherwise keeps its live base
        // fee in the reset genesis; returning from a fork restores the local default instead of
        // leaking remote fee state. Compute this up front so the env, storage, and fee manager all
        // agree.
        let preserve_live_base_fee =
            !reset_from_fork && !local_base_fee_is_explicit && local_tempo_hardfork.is_none();
        let genesis_base_fee =
            if preserve_live_base_fee { staged_fees.base_fee() } else { local_base_fee };

        let mut staged_cfg = CfgEnv::default();
        staged_config.set_cfg_spec(&mut staged_cfg, local_spec);
        staged_cfg.chain_id = local_chain_id;
        staged_cfg.limit_contract_code_size = staged_config.code_size_limit;
        staged_cfg.disable_eip3607 = true;
        staged_cfg.disable_block_gas_limit = staged_config.disable_block_gas_limit;
        if let Some(memory_limit) = staged_config.memory_limit {
            staged_cfg.memory_limit = memory_limit;
        }
        let staged_env = EvmEnv::new(
            staged_cfg,
            BlockEnv {
                number: U256::from(genesis_number),
                beneficiary: local_beneficiary,
                timestamp: U256::from(genesis_timestamp),
                gas_limit: local_gas_limit,
                basefee: genesis_base_fee,
                prevrandao: Some(B256::ZERO),
                ..Default::default()
            },
        );

        let base_fee = staged_fees.is_eip1559().then_some(genesis_base_fee);
        let header = genesis_header(&staged_env, base_fee, genesis_timestamp, genesis_number);
        let staged_storage = BlockchainStorage::new(foundry_header(&self.networks, header));

        // Seed the next block's fee state. Tempo always advances through its hardfork rule, an
        // implicit in-memory Ethereum reset restores Anvil's default, and explicit or
        // fork-to-memory Ethereum resets retain the local configured value.
        staged_fees.set_base_fee(genesis_base_fee);
        if staged_fees.is_eip1559() {
            let next_base_fee = if staged_fees.tempo_hardfork().is_some() {
                staged_fees.get_next_block_base_fee_per_gas(
                    0,
                    staged_env.block_env.gas_limit,
                    genesis_base_fee,
                )
            } else if preserve_live_base_fee {
                crate::eth::fees::INITIAL_BASE_FEE
            } else {
                genesis_base_fee
            };
            staged_fees.set_base_fee(next_base_fee);
        }
        staged_fees.set_gas_price(local_gas_price);

        let mut staged_db: Box<dyn Db> = Box::new(StateRootDb::new(
            self.prune_state_history_config.is_state_history_supported(),
        ));
        Self::populate_memory_db(
            &mut *staged_db,
            &self.genesis,
            &staged_config.funded_accounts,
            local_hardfork,
            local_chain_id,
            self.networks.is_tempo(),
            local_tempo_hardfork,
            staged_storage.genesis_hash,
            install_create2_deployer,
        )?;
        #[cfg(feature = "base")]
        if self.is_base() {
            let upgrade = match local_hardfork {
                FoundryHardfork::Base(upgrade) => upgrade,
                _ => BaseUpgrade::Azul,
            };
            Self::apply_base_genesis(&mut *staged_db, upgrade)?;
        }

        Ok(StagedMemoryReset {
            node_config: staged_config,
            db: staged_db,
            fees: staged_fees,
            evm_env: staged_env,
            hardfork: local_hardfork,
            storage: staged_storage,
            timestamp: genesis_timestamp,
            flush_old_cache: reset_from_fork,
        })
    }

    /// Atomically publishes a fully prepared in-memory replacement.
    pub(crate) async fn commit_memory_reset(
        &self,
        staged: StagedMemoryReset<N>,
    ) -> Result<(), BlockchainError> {
        let StagedMemoryReset {
            node_config,
            db,
            fees,
            evm_env,
            hardfork,
            mut storage,
            timestamp,
            flush_old_cache,
        } = staged;
        let mut live_config = self.node_config.write().await;
        let mut live_db = self.db.write().await;
        if flush_old_cache {
            live_db.maybe_flush_cache().map_err(BlockchainError::Internal)?;
        }

        *live_config = node_config;
        *live_db = db;
        self.fees.replace_from(&fees);
        *self.evm_env.write() = evm_env;
        *self.hardfork.write() = hardfork;
        *self.fork.write() = None;
        {
            let mut live_storage = self.blockchain.storage.write();
            storage.tempo = live_storage.tempo.take();
            if let Some(tempo) = &mut storage.tempo {
                tempo.reanchor(storage.best_number);
            }
            *live_storage = storage;
        }
        self.states.write().clear();
        self.active_state_snapshots.lock().clear();
        self.time.reset(timestamp);
        self.cheats.clear_next_block_overrides();
        trace!(target: "backend", "reset to fresh in-memory state");
        Ok(())
    }

    /// Reverts the state to the state snapshot identified by the given `id`.
    pub async fn revert_state_snapshot(&self, id: U256) -> Result<bool, BlockchainError>
    where
        N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
    {
        let Some((num, hash, fees, time, next_block)) =
            self.active_state_snapshots.lock().get(&id).map(|snapshot| {
                (
                    snapshot.block_number,
                    snapshot.block_hash,
                    snapshot.fees,
                    snapshot.time,
                    snapshot.next_block,
                )
            })
        else {
            return Ok(false);
        };
        let block = self.block_by_hash(hash).await?.ok_or(BlockchainError::BlockNotFound)?;
        let removed_logs = self.removed_logs_since(num);
        let mut db = self.db.write().await;
        if !db.revert_state(id, RevertStateSnapshotAction::RevertRemove) {
            return Ok(false);
        }
        {
            let mut snapshots = self.active_state_snapshots.lock();
            snapshots.remove(&id);
            snapshots.retain(|snapshot_id, _| *snapshot_id < id);
        }
        // Revert the storage that's newer than the snapshot.
        let (removed_blocks, proof_state) = {
            let mut storage = self.blockchain.storage.write();
            let proof_state = storage.tempo.as_mut().and_then(|tempo| {
                tempo.restore(id, num).map(|state| (Arc::clone(&tempo.proof_states), state))
            });
            (storage.unwind_to(num, hash), proof_state)
        };
        if let Some((states, accounts)) = proof_state {
            let mut state = CacheDB::new(EmptyDb);
            state.cache.accounts = (*accounts).clone();
            let mut states = states.write();
            states.remove_block_states(&[hash]);
            states.insert(hash, StateDb::new(state));
        }
        let removed_hashes: Vec<_> = removed_blocks.iter().map(|b| b.header.hash_slow()).collect();
        self.states.write().remove_block_states(&removed_hashes);
        if let Some(states) = self.tempo_proof_states() {
            states.write().remove_block_states(&removed_hashes);
        }
        if !removed_logs.is_empty() {
            self.notify_on_removed_logs(removed_logs);
        }

        self.time.restore(time);
        self.cheats.restore_next_block_overrides(&next_block);

        {
            let mut env = self.evm_env.write();
            env.block_env = BlockEnv {
                number: self.evm_block_number(num),
                timestamp: U256::from(block.header.timestamp()),
                difficulty: block.header.difficulty(),
                // ensures prevrandao is set
                prevrandao: Some(block.header.mix_hash().unwrap_or_default()),
                gas_limit: block.header.gas_limit(),
                // Keep previous `beneficiary` and `basefee` value
                beneficiary: env.block_env.beneficiary,
                basefee: env.block_env.basefee,
                ..Default::default()
            };
        }
        self.fees.restore(fees);
        Ok(true)
    }

    fn tempo_proof_states(&self) -> Option<Arc<RwLock<InMemoryBlockStates>>> {
        self.blockchain.storage.read().tempo.as_ref().map(|tempo| Arc::clone(&tempo.proof_states))
    }
}

impl<N: Network> Backend<N>
where
    N::ReceiptEnvelope: TxReceipt<Log = alloy_primitives::Log>,
{
    /// Appends all `Log`s mined by the node that were emitted in the `block` and match the
    /// `Filter`.
    fn mined_logs_for_block(
        storage: &BlockchainStorage<N>,
        filter: &Filter,
        block: &Block,
        block_hash: B256,
        all_logs: &mut Vec<Log>,
    ) {
        if !filter.matches_bloom(block.header.logs_bloom) {
            return;
        }

        let mut block_log_index = 0u32;
        for tx in &block.body.transactions {
            let Some(tx) = storage.transactions.get(&tx.hash()) else {
                continue;
            };

            let logs = tx.receipt.logs();
            let transaction_hash = tx.info.transaction_hash;

            for log in logs {
                if filter.matches(log) {
                    all_logs.push(Log {
                        inner: log.clone(),
                        block_hash: Some(block_hash),
                        block_number: Some(block.header.number()),
                        block_timestamp: Some(block.header.timestamp()),
                        transaction_hash: Some(transaction_hash),
                        transaction_index: Some(tx.info.transaction_index),
                        log_index: Some(block_log_index as u64),
                        removed: false,
                    });
                }
                block_log_index += 1;
            }
        }
    }

    /// Returns all logs of the blocks with a number greater than `block_number`, marked as
    /// removed.
    ///
    /// This captures the logs of blocks that are about to be unwound before their transactions
    /// and receipts are cleared from storage, so they can be re-delivered to log subscriptions
    /// and filters with `removed: true`.
    fn removed_logs_since(&self, block_number: u64) -> Vec<Log> {
        let storage = self.blockchain.storage.read();
        let mut all_logs = Vec::new();

        for num in (block_number + 1)..=storage.best_number {
            if let Some(hash) = storage.hashes.get(&num)
                && let Some(block) = storage.blocks.get(hash)
            {
                let mut block_log_index = 0u64;
                for tx in &block.body.transactions {
                    if let Some(tx) = storage.transactions.get(&tx.hash()) {
                        for log in tx.receipt.logs() {
                            all_logs.push(Log {
                                inner: log.clone(),
                                block_hash: Some(*hash),
                                block_number: Some(num),
                                block_timestamp: Some(block.header.timestamp()),
                                transaction_hash: Some(tx.info.transaction_hash),
                                transaction_index: Some(tx.info.transaction_index),
                                log_index: Some(block_log_index),
                                removed: true,
                            });
                            block_log_index += 1;
                        }
                    }
                }
            }
        }

        all_logs
    }

    /// Returns the logs of the block that match the filter
    async fn logs_for_block(
        &self,
        filter: Filter,
        hash: B256,
    ) -> Result<Vec<Log>, BlockchainError> {
        {
            let storage = self.blockchain.storage.read();
            if let Some(block) = storage.blocks.get(&hash) {
                let mut logs = Vec::new();
                Self::mined_logs_for_block(&storage, &filter, block, hash, &mut logs);
                return Ok(logs);
            }
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.logs(&filter).await?);
        }

        Err(BlockchainError::UnknownBlock)
    }

    /// Returns the logs that match the filter in the given range of blocks
    async fn logs_for_range(
        &self,
        filter: &Filter,
        mut from: u64,
        to: u64,
    ) -> Result<Vec<Log>, BlockchainError> {
        let mut all_logs = Vec::new();

        // get the range that predates the fork if any
        if let Some(fork) = self.get_fork() {
            let to_on_fork = if fork.predates_fork(to) {
                to
            } else {
                // adjust the ranges
                fork.block_number()
            };

            if fork.predates_fork_inclusive(from) {
                // this data is only available on the forked client
                let filter = filter.clone().from_block(from).to_block(to_on_fork);
                all_logs = fork.logs(&filter).await?;

                // update the range
                from = fork.block_number() + 1;
            }
        }

        let storage = self.blockchain.storage.read();
        for number in from..=to {
            if let Some(&hash) = storage.hashes.get(&number)
                && let Some(block) = storage.blocks.get(&hash)
            {
                Self::mined_logs_for_block(&storage, filter, block, hash, &mut all_logs);
            }
        }

        Ok(all_logs)
    }

    /// Returns the logs according to the filter
    pub async fn logs(&self, filter: Filter) -> Result<Vec<Log>, BlockchainError> {
        trace!(target: "backend", "get logs [{:?}]", filter);
        if let Some(hash) = filter.get_block_hash() {
            self.logs_for_block(filter, hash).await
        } else {
            let best = self.best_number();
            let to_block = self
                .convert_block_number_at(filter.block_option.get_to_block().copied(), best)
                .min(best);
            let from_block =
                self.convert_block_number_at(filter.block_option.get_from_block().copied(), best);
            if from_block > best {
                return Err(BlockchainError::BlockOutOfRange(best, from_block));
            }
            if from_block > to_block {
                return Err(BlockchainError::RpcError(RpcError::invalid_params(
                    "invalid block range params",
                )));
            }

            self.logs_for_range(&filter, from_block, to_block).await
        }
    }

    /// Returns all receipts of the block
    pub fn mined_receipts(&self, hash: B256) -> Option<Vec<N::ReceiptEnvelope>> {
        let storage = self.blockchain.storage.read();
        let block = storage.blocks.get(&hash)?;
        block
            .body
            .transactions
            .iter()
            .map(|transaction| {
                storage.transactions.get(&transaction.hash()).map(|tx| tx.receipt.clone())
            })
            .collect()
    }

    /// Returns the EIP-7928 block access list stored for a locally mined block hash.
    pub fn block_access_list_by_hash(&self, hash: B256) -> Option<BlockAccessList> {
        self.blockchain.storage.read().block_access_lists.get(&hash).cloned()
    }

    /// Returns the EIP-7928 block access list stored for a locally mined block number.
    pub fn block_access_list_by_number(&self, number: u64) -> Option<BlockAccessList> {
        let storage = self.blockchain.storage.read();
        storage.hashes.get(&number).and_then(|hash| storage.block_access_lists.get(hash)).cloned()
    }
}

// Mining methods — generic over N: Network, with Foundry-associated-type bounds for now.
impl<N: Network> Backend<N>
where
    Self: TransactionValidator<FoundryTxEnvelope>,
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    /// Mines a new block and stores it.
    ///
    /// this will execute all transaction in the order they come in and return all the markers they
    /// provide.
    pub async fn mine_block(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
    ) -> Result<MinedBlockOutcome<FoundryTxEnvelope>, BlockchainError> {
        self.do_mine_block(pool_transactions).await
    }

    /// Mines a new block while the caller holds the mining lock.
    pub(crate) async fn mine_block_locked(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
        pin_timestamp: bool,
    ) -> Result<MinedBlockOutcome<FoundryTxEnvelope>, BlockchainError> {
        self.do_mine_block_locked(pool_transactions, pin_timestamp).await
    }

    /// Replays a transaction-hash fork prefix before the live pool and miner are created.
    pub(crate) async fn apply_fork_transaction_replay(
        &self,
        replay: ForkTransactionReplay,
    ) -> Result<()> {
        let source_chain_id = self.protocol_chain_id();
        let arbitrum_block_numbers = is_arbitrum(source_chain_id).then(|| {
            (
                replay.source_block.header().number(),
                arbitrum_replay_block_number(&replay.source_block),
            )
        });
        let prepared = prepare_fork_transaction_replay(replay, self.is_monad(), self.is_celo())?;
        let fallback_execution_chain_id = self
            .get_fork()
            .map(|fork| fork.execution_chain_id())
            .unwrap_or_else(|| self.chain_id().to());
        let execution_chain_id = prepared.execution_chain_id(fallback_execution_chain_id)?;
        let PreparedForkTransactionReplay {
            transactions,
            beneficiary,
            timestamp,
            parent_beacon_block_root,
        } = prepared;
        eyre::ensure!(!transactions.is_empty(), "fork transaction replay prefix is empty");
        let next_timestamp = timestamp.checked_add(1).ok_or_else(|| {
            eyre::eyre!("fork transaction replay timestamp cannot be incremented")
        })?;

        let _mining_guard = self.mining.lock().await;
        let current_base_fee = self.base_fee();
        let current_excess_blob_gas_and_price = self.excess_blob_gas_and_price();
        let mut evm_env = self.evm_env.read().clone();
        if evm_env.block_env.basefee == 0 {
            evm_env.cfg_env.disable_base_fee = true;
        }

        let best_number = self.blockchain.storage.read().best_number;
        let block_number = best_number.saturating_add(1);
        evm_env.block_env.number = evm_env.block_env.number.saturating_add(U256::ONE);
        evm_env.block_env.basefee = current_base_fee;
        evm_env.block_env.blob_excess_gas_and_price = current_excess_blob_gas_and_price;
        evm_env.block_env.timestamp = U256::from(timestamp);

        let best_hash = self.blockchain.storage.read().best_hash;
        let mut prevrandao_input = [0u8; 40];
        prevrandao_input[..32].copy_from_slice(best_hash.as_slice());
        prevrandao_input[32..].copy_from_slice(&block_number.to_le_bytes());
        evm_env.block_env.prevrandao = Some(
            self.cheats.take_next_block_prevrandao().unwrap_or_else(|| keccak256(prevrandao_input)),
        );

        let mut replay_env = evm_env.clone();
        // Execute the prefix with the source block's coinbase so priority fees, COINBASE, and the
        // header miner match the source. Later blocks keep the local beneficiary.
        replay_env.block_env.beneficiary = beneficiary;
        let arbitrum_rpc_block_number = arbitrum_block_numbers.map(|(rpc, evm)| {
            replay_env.block_env.number = evm;
            rpc
        });
        replay_env.cfg_env.chain_id = execution_chain_id;
        apply_chain_specific_tx_replay_env_changes_for_chain(&mut replay_env, source_chain_id);
        let inspector_tx_config = self.inspector_tx_config();

        let scheduled_hardfork =
            source_hardfork(self.networks.execution_network(), source_chain_id, timestamp);
        #[cfg(feature = "monad")]
        let mut monad_replay = self
            .prepare_monad_fork_replay(
                source_chain_id,
                execution_chain_id,
                timestamp,
                best_hash,
                &transactions,
            )
            .await?;
        #[cfg(feature = "monad")]
        let hardfork = monad_replay
            .as_ref()
            .map(monad::ForkReplay::hardfork)
            .unwrap_or_else(|| scheduled_hardfork.unwrap_or_else(|| self.hardfork()));
        #[cfg(not(feature = "monad"))]
        let hardfork = scheduled_hardfork.unwrap_or_else(|| self.hardfork());
        if !self.is_optimism() && !self.is_tempo() {
            replay_env.cfg_env.spec = SpecId::from(hardfork);
        }
        let replay_spec = SpecId::from(hardfork);
        // Cancun-family execution requires blob excess gas even for non-blob transactions. Some
        // OP Stack source headers omit it, so use the neutral value for historical replay.
        if !self.is_tempo()
            && replay_spec >= SpecId::CANCUN
            && replay_env.block_env.blob_excess_gas_and_price.is_none()
        {
            replay_env.block_env.blob_excess_gas_and_price = Some(BlobExcessGasAndPrice::new(
                0,
                get_blob_base_fee_update_fraction_by_spec_id(replay_spec),
            ));
        }

        #[cfg(feature = "monad")]
        let monad_context = monad_replay.as_mut().and_then(monad::ForkReplay::take_context);
        #[cfg(not(feature = "monad"))]
        let monad_context = None;

        let (block_info, state_changes, block_hash) = {
            let db = self.db.read().await;
            let mut overlay = CacheDB::new(&**db);
            let ExecutedHistoricalReplay {
                block_result,
                transactions,
                transaction_infos,
                state_changes,
            } = self.execute_with_replay_block_executor(
                &overlay,
                &replay_env,
                best_hash,
                arbitrum_rpc_block_number,
                hardfork,
                parent_beacon_block_root,
                &transactions,
                &inspector_tx_config,
                monad_context,
            )?;
            overlay.commit(&state_changes);
            let state_root = overlay.maybe_state_root().unwrap_or_default();
            let block_info = self.build_block_info(
                &replay_env,
                best_hash,
                block_number,
                state_root,
                block_result,
                transactions,
                transaction_infos,
                parent_beacon_block_root,
                None,
            );
            let block_hash = block_info.block.header.hash_slow();
            (block_info, state_changes, block_hash)
        };

        if self.prune_state_history_config.is_state_history_supported() {
            let state = self.db.read().await.current_state();
            self.states.write().insert(best_hash, state);
        }

        let mut db = self.db.write().await;
        db.commit(&state_changes);
        db.insert_block_hash(U256::from(block_number), block_hash);

        let BlockInfo { block, transactions, receipts } = block_info;
        let header = block.header.clone();
        {
            let mut storage = self.blockchain.storage.write();
            storage.best_number = block_number;
            storage.best_hash = block_hash;
            if let Some(tempo) = &mut storage.tempo {
                tempo.reanchor(block_number);
            }
            if !self.is_eip3675() {
                storage.total_difficulty =
                    storage.total_difficulty.saturating_add(header.difficulty);
            }
            storage.blocks.insert(block_hash, block);
            storage.hashes.insert(block_number, block_hash);
            #[cfg(feature = "monad")]
            if let Some(replay) = &mut monad_replay {
                replay.store_metadata(&mut storage, block_hash);
            }
            for (info, receipt) in transactions.into_iter().zip(receipts) {
                let mined_tx = MinedTransaction { info, receipt, block_hash, block_number };
                storage.transactions.insert(mined_tx.info.transaction_hash, mined_tx);
            }

            if let Some(transaction_block_keeper) = self.transaction_block_keeper
                && storage.blocks.len() > transaction_block_keeper
            {
                let to_clear = block_number
                    .saturating_sub(transaction_block_keeper.try_into().unwrap_or(u64::MAX));
                storage.remove_block_transactions_by_number(to_clear)
            }
        }

        #[cfg(feature = "monad")]
        if let Some(replay) = &monad_replay {
            self.finalize_monad_fork_replay(replay, &mut evm_env);
        }

        evm_env.block_env.difficulty = U256::ZERO;
        *self.evm_env.write() = evm_env;
        self.time.reset(timestamp);
        self.time.set_next_block_timestamp(next_timestamp)?;

        #[cfg(feature = "optimism")]
        if self.is_optimism() {
            self.fees.set_optimism_base_fee_rules(header.extra_data());
        }
        let next_block_base_fee = self.fees.get_next_block_base_fee_from_header(&header);
        let blob_params = self.fees.blob_params();
        let next_block_excess_blob_gas = self.networks.next_block_blob_excess_gas(
            blob_params,
            header.excess_blob_gas.unwrap_or_default(),
            header.blob_gas_used.unwrap_or_default(),
            header.base_fee_per_gas.unwrap_or_default(),
        );
        self.fees.set_base_fee(next_block_base_fee);
        self.fees.set_blob_excess_gas_and_price(BlobExcessGasAndPrice::new(
            next_block_excess_blob_gas,
            blob_params.update_fraction as u64,
        ));
        drop(db);
        self.notify_on_new_block(header.into_inner(), block_hash);

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_with_replay_block_executor(
        &self,
        db: &dyn DatabaseRef,
        evm_env: &EvmEnv,
        parent_hash: B256,
        arbitrum_rpc_block_number: Option<u64>,
        hardfork: FoundryHardfork,
        parent_beacon_block_root: Option<B256>,
        transactions: &[HistoricalReplayTransaction],
        inspector_tx_config: &InspectorTxConfig,
        #[cfg_attr(not(feature = "monad"), allow(unused_variables))] monad_context: Option<
            MonadReplayContext,
        >,
    ) -> Result<ExecutedHistoricalReplay> {
        #[cfg(feature = "monad")]
        let mut monad_context = if self.is_monad() {
            Some(
                monad_context
                    .ok_or_else(|| eyre::eyre!("Monad replay ancestor context is unavailable"))?,
            )
        } else {
            None
        };
        let transitions = self.ethereum_block_transitions(
            hardfork,
            parent_beacon_block_root,
            BlockExecutionKind::TransactionPrefix,
        );
        let mut opts = self.evm_options_at(evm_env, hardfork);
        if let Some(block_number) = arbitrum_rpc_block_number {
            opts.arbitrum_block_number = Some(block_number);
        }
        with_evm_network!(self.evm_family(), E => {
            // Historical replay has no local blob budget.
            let mut executor = self.block_executor::<E>(
                db,
                evm_env,
                &opts,
                parent_hash,
                transitions,
                self.build_mining_inspector(),
                u64::MAX,
            )?;
            executor
                .apply_pre_execution_changes()
                .wrap_err("failed to apply replay block-start transitions")?;
            let (stored_transactions, transaction_infos) = execute_historical_replay(
                &mut executor,
                transactions,
                inspector_tx_config,
                &mut |_replay| {
                    #[cfg(feature = "monad")]
                    if let Some(context) = monad_context.as_mut() {
                        let tx = PreparedTx::signed(
                            _replay.transaction.inner().clone(),
                            _replay.transaction.signer(),
                        );
                        return TxContext {
                            monad: monad::resolve_execution_context(
                                Some(MonadExecutionContext::Next(context)),
                                &tx,
                                self.cheats(),
                            ),
                            // The source block is trusted history; only envelopes the source
                            // attributes to the system sender are prepared with it.
                            monad_protocol_replay: true,
                        };
                    }
                    TxContext::default()
                },
            )?;
            let (block_result, state_changes, _) =
                executor.finish().wrap_err("failed to finish replay block execution")?;
            Ok(ExecutedHistoricalReplay {
                block_result,
                transactions: stored_transactions,
                transaction_infos,
                state_changes,
            })
        })
    }

    /// Builds a [`BlockInfo`] from the EVM environment, execution results, and transactions.
    #[allow(clippy::too_many_arguments)]
    fn build_block_info(
        &self,
        evm_env: &EvmEnv,
        parent_hash: B256,
        number: u64,
        state_root: B256,
        block_result: BlockExecutionResult,
        transactions: Vec<MaybeImpersonatedTransaction<FoundryTxEnvelope>>,
        transaction_infos: Vec<TransactionInfo>,
        parent_beacon_block_root: Option<B256>,
        block_access_list: Option<&BlockAccessList>,
    ) -> BlockInfo<N> {
        let spec_id = *evm_env.spec_id();
        let is_shanghai = spec_id >= SpecId::SHANGHAI;
        let is_cancun = spec_id >= SpecId::CANCUN;
        let is_prague = spec_id >= SpecId::PRAGUE;

        let receipts_root = calculate_receipt_root(&block_result.receipts);
        let cumulative_blob_gas_used = is_cancun.then_some(block_result.blob_gas_used);
        let bloom = block_result.receipts.iter().fold(Bloom::default(), |mut b, r| {
            b.accrue_bloom(r.logs_bloom());
            b
        });

        let header = Header {
            parent_hash,
            ommers_hash: Default::default(),
            beneficiary: evm_env.block_env.beneficiary,
            state_root,
            transactions_root: Default::default(),
            receipts_root,
            logs_bloom: bloom,
            difficulty: evm_env.block_env.difficulty,
            number,
            gas_limit: evm_env.block_env.gas_limit,
            gas_used: block_result.gas_used,
            timestamp: evm_env.block_env.timestamp.saturating_to(),
            extra_data: self.fees.base_fee_extra_data(),
            mix_hash: evm_env.block_env.prevrandao.unwrap_or_default(),
            nonce: Default::default(),
            base_fee_per_gas: (spec_id >= SpecId::LONDON).then_some(evm_env.block_env.basefee),
            parent_beacon_block_root: is_cancun
                .then(|| parent_beacon_block_root.unwrap_or_default()),
            blob_gas_used: cumulative_blob_gas_used,
            excess_blob_gas: if is_cancun { evm_env.block_env.blob_excess_gas() } else { None },
            withdrawals_root: is_shanghai.then_some(EMPTY_WITHDRAWALS),
            requests_hash: is_prague.then(|| block_result.requests.requests_hash()),
            block_access_list_hash: block_access_list
                .map(|bal| compute_block_access_list_hash(bal.as_slice())),
            slot_number: None,
        };

        let block = create_block(foundry_header(&self.networks, header), transactions);
        BlockInfo { block, transactions: transaction_infos, receipts: block_result.receipts }
    }

    async fn do_mine_block(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
    ) -> Result<MinedBlockOutcome<FoundryTxEnvelope>, BlockchainError> {
        let _mining_guard = self.mining.lock().await;
        self.do_mine_block_locked(pool_transactions, false).await
    }

    /// Mines a block while the caller holds the mining lock.
    async fn do_mine_block_locked(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
        pin_timestamp: bool,
    ) -> Result<MinedBlockOutcome<FoundryTxEnvelope>, BlockchainError> {
        trace!(target: "backend", "creating new block with {} transactions", pool_transactions.len());

        let (outcome, header, block_hash) = {
            let current_base_fee = self.base_fee();
            let current_excess_blob_gas_and_price = self.excess_blob_gas_and_price();

            let mut evm_env = self.evm_env.read().clone();
            #[cfg(feature = "monad")]
            let execution_chain_id = evm_env.cfg_env.chain_id;
            let hardfork = self.hardfork();

            if evm_env.block_env.basefee == 0 {
                // this is an edge case because the evm fails if `tx.effective_gas_price < base_fee`
                // 0 is only possible if it's manually set
                evm_env.cfg_env.disable_base_fee = true;
            }

            let block_number = self.blockchain.storage.read().best_number.saturating_add(1);

            // Advance the EVM number independently of the RPC header number. On Arbitrum
            // forks this preserves the L1 number read by NUMBER while the header tracks L2.
            evm_env.block_env.number = evm_env.block_env.number.saturating_add(U256::ONE);

            evm_env.block_env.basefee = current_base_fee;
            evm_env.block_env.blob_excess_gas_and_price = current_excess_blob_gas_and_price;

            let best_hash = self.blockchain.storage.read().best_hash;

            let mut input = [0u8; 40];
            input[..32].copy_from_slice(best_hash.as_slice());
            input[32..].copy_from_slice(&block_number.to_le_bytes());
            // Use the values set via `anvil_setNextBlockPrevRandao` and
            // `anvil_setNextBlockParentBeaconBlockRoot` for this block if provided, otherwise
            // derive `prevrandao` from the parent hash and block number and use the zero root.
            // The overrides are consumed once the block is committed so they only apply to it.
            let next_block = self.cheats.next_block_overrides();
            evm_env.block_env.prevrandao =
                Some(next_block.prevrandao.value.unwrap_or_else(|| keccak256(input)));
            let parent_beacon_block_root =
                Some(next_block.parent_beacon_block_root.value.unwrap_or_default());

            let (
                db_guard,
                block_info,
                included,
                stale,
                invalid,
                not_yet_valid,
                block_hash,
                parent_state,
                next_block_base_fee,
                next_block_blob_fees,
                block_access_list,
            ) = {
                let mut db = self.db.write().await;

                // finally set the next block timestamp, this is done just before execution, because
                // there can be concurrent requests that can delay acquiring the db lock and we want
                // to ensure the timestamp is as close as possible to the actual execution.
                let pending_timestamp = self.prepare_block_timestamp(
                    &**db,
                    evm_env.block_env.timestamp.saturating_to(),
                    pin_timestamp,
                )?;
                let block_timestamp = pending_timestamp.timestamp;
                evm_env.block_env.timestamp = U256::from(block_timestamp);

                // Forced historical transactions bypass pool admission and are replayed while
                // mining. Keep this exception local to the disposable mining environment.
                let mut mining_evm_env = evm_env.clone();
                if pool_transactions.iter().any(|tx| tx.is_replay) {
                    apply_chain_specific_tx_replay_env_changes_for_chain(
                        &mut mining_evm_env,
                        self.protocol_chain_id(),
                    );
                }

                #[cfg(feature = "base")]
                let (protocol_transactions, system_transaction_count) = {
                    let mut transactions = self.base_system_transactions(
                        &**db,
                        block_number,
                        block_timestamp,
                        best_hash,
                    )?;
                    let system_transaction_count = transactions.len();
                    transactions.extend(pool_transactions);
                    (transactions, system_transaction_count)
                };
                #[cfg(not(feature = "base"))]
                let protocol_transactions = pool_transactions;

                let inspector_tx_config = self.inspector_tx_config();
                let gas_config = self.pool_tx_gas_config(&mining_evm_env);

                let ExecutedBlock {
                    pool: pool_result,
                    result: block_result,
                    state: block_state,
                    bal: block_access_list,
                } = self.execute_with_block_executor(
                    &**db,
                    &mining_evm_env,
                    best_hash,
                    hardfork,
                    parent_beacon_block_root,
                    BlockExecutionKind::Complete,
                    &protocol_transactions,
                    &gas_config,
                    &inspector_tx_config,
                    &|pool_tx, account| {
                        let validation_env =
                            if pool_tx.is_replay { &mining_evm_env } else { &evm_env };
                        self.validate_mining_pool_transaction_for(pool_tx, account, validation_env)
                    },
                )?;

                #[cfg(feature = "base")]
                base::validate_system_transactions(
                    &protocol_transactions[..system_transaction_count],
                    &pool_result,
                )?;

                #[cfg_attr(not(feature = "base"), allow(unused_mut))]
                let mut included = pool_result.included;
                // System deposits are generated per block rather than taken from the pool. Report
                // only pool transactions so the pool does not treat every block as progress.
                #[cfg(feature = "base")]
                included.drain(..system_transaction_count);
                let stale = pool_result.stale;
                let invalid = pool_result.invalid;
                let not_yet_valid = pool_result.not_yet_valid;

                let parent_state = self
                    .prune_state_history_config
                    .is_state_history_supported()
                    .then(|| db.current_state());
                let mut block_info = self.build_block_info(
                    &mining_evm_env,
                    best_hash,
                    block_number,
                    B256::ZERO,
                    block_result,
                    pool_result.txs,
                    pool_result.tx_info,
                    parent_beacon_block_root,
                    block_access_list.as_ref(),
                );

                if let FoundryHeader::Tempo(header) = &mut block_info.block.header {
                    let storage = self.blockchain.storage.read();
                    let tempo = storage.tempo.as_ref().expect("Tempo chain is initialized");
                    tempo.prepare(header, TempoHardfork::from(hardfork))?;
                }

                // The database guard protects the prepared consensus state until publication.
                // Keep the expensive execution commit and state root outside the storage lock.
                db.commit(&block_state);
                #[cfg(test)]
                if let Some(hook) = self.mining_state_root_hook.lock().as_ref() {
                    hook();
                }
                let proof_state = (matches!(block_info.block.header, FoundryHeader::Tempo(_))
                    && self.get_fork().is_some())
                .then(|| {
                    let mut state = CacheDB::new(EmptyDb);
                    state.cache.accounts = db
                        .maybe_as_full_db()
                        .expect("a fork database exposes its cached accounts")
                        .clone();
                    state
                });
                let state_root = proof_state.as_ref().map_or_else(
                    || db.maybe_state_root().unwrap_or_default(),
                    |state| {
                        let proof_root = Arc::clone(
                            &self
                                .blockchain
                                .storage
                                .read()
                                .tempo
                                .as_ref()
                                .expect("Tempo chain is initialized")
                                .proof_root,
                        );
                        let (cache, previous) = &mut *proof_root.lock();
                        // Rehash only the paths that changed since the previous seal.
                        cache.record_diff(&state.cache.accounts, previous);
                        let root = cache.root(&state.cache.accounts);
                        previous.clone_from(&state.cache.accounts);
                        root
                    },
                );
                match &mut block_info.block.header {
                    FoundryHeader::Tempo(header) => header.inner.state_root = state_root,
                    FoundryHeader::Ethereum(header) => header.state_root = state_root,
                }

                // Update the new blockhash in the db itself.
                let block_hash = block_info.block.header.hash_slow();
                if let Some(proof_state) = proof_state {
                    let proof_states =
                        self.tempo_proof_states().expect("Tempo chain is initialized");
                    let mut proof_states = proof_states.write();
                    // Re-mining after a rewind can produce an already cached hash.
                    proof_states.remove_block_states(&[block_hash]);
                    proof_states.insert(block_hash, StateDb::new(proof_state));
                }
                db.insert_block_hash(U256::from(block_info.block.header.number()), block_hash);
                self.time.commit_next_timestamp(pending_timestamp);
                self.cheats.consume_next_block_overrides(&next_block);

                let header = &block_info.block.header;
                let next_block_base_fee = self.fees.get_next_block_base_fee_from_header(header);
                let next_block_excess_blob_gas = self.networks.next_block_blob_excess_gas(
                    self.fees.blob_params(),
                    header.excess_blob_gas.unwrap_or_default(),
                    header.blob_gas_used.unwrap_or_default(),
                    header.base_fee_per_gas.unwrap_or_default(),
                );
                let next_block_blob_fees = BlobExcessGasAndPrice::new(
                    next_block_excess_blob_gas,
                    self.fees.blob_params().update_fraction as u64,
                );

                (
                    db,
                    block_info,
                    included,
                    stale,
                    invalid,
                    not_yet_valid,
                    block_hash,
                    parent_state,
                    next_block_base_fee,
                    next_block_blob_fees,
                    block_access_list,
                )
            };

            // create the new block with the current timestamp
            let BlockInfo { block, transactions, receipts } = block_info;

            let header = block.header.clone();
            #[cfg(feature = "monad")]
            let monad_participants = self.is_monad().then(|| {
                monad::block_participants(
                    included.iter().map(|pool_tx| &pool_tx.pending_transaction),
                    self.cheats(),
                )
            });

            if let Some(parent_state) = parent_state {
                self.states.write().insert(best_hash, parent_state);
            }

            trace!(
                target: "backend",
                "Mined block {} with {} tx {:?}",
                block_number,
                transactions.len(),
                transactions.iter().map(|tx| tx.transaction_hash).collect::<Vec<_>>()
            );
            #[cfg(test)]
            let mining_commit_hook = { self.mining_commit_hook.lock().clone() };
            #[cfg(test)]
            if let Some(hook) = mining_commit_hook {
                hook.reached.notify_one();
                hook.resume.notified().await;
            }
            let mut storage = self.blockchain.storage.write();
            if let FoundryHeader::Tempo(header) = &header {
                storage
                    .tempo
                    .as_mut()
                    .expect("Tempo chain is initialized")
                    .finalize(header, TempoHardfork::from(hardfork));
            }
            // update block metadata
            storage.best_number = block_number;
            storage.best_hash = block_hash;
            // Difficulty is removed and not used after Paris (aka TheMerge). Value is replaced with
            // prevrandao. https://github.com/bluealloy/revm/blob/1839b3fce8eaeebb85025576f2519b80615aca1e/crates/interpreter/src/instructions/host_env.rs#L27
            if !self.is_eip3675() {
                storage.total_difficulty =
                    storage.total_difficulty.saturating_add(header.difficulty);
            }

            if let Some(tempo) = &mut storage.tempo {
                tempo.publish(&block);
            }
            storage.blocks.insert(block_hash, block);
            storage.hashes.insert(block_number, block_hash);
            if let Some(block_access_list) = block_access_list {
                storage.block_access_lists.insert(block_hash, block_access_list);
            }
            #[cfg(feature = "monad")]
            if let Some(participants) = monad_participants {
                monad::store_block_metadata(
                    &mut storage,
                    block_hash,
                    participants,
                    execution_chain_id,
                    hardfork,
                );
            }

            node_info!("");
            // insert all transactions
            for (info, receipt) in transactions.into_iter().zip(receipts) {
                // log some tx info
                node_info!("    Transaction: {:?}", info.transaction_hash);
                if let Some(contract) = &info.contract_address {
                    node_info!("    Contract created: {contract}");
                }
                node_info!("    Gas used: {}", info.gas_used);
                if !info.exit.is_success() {
                    let r = RevertDecoder::new().decode(
                        info.out.as_ref().map(|b| &b[..]).unwrap_or_default(),
                        Some(info.exit),
                    );
                    node_info!("    Error: reverted with: {r}");
                }
                node_info!("");

                let mined_tx = MinedTransaction { info, receipt, block_hash, block_number };
                storage.transactions.insert(mined_tx.info.transaction_hash, mined_tx);
            }

            // remove old transactions that exceed the transaction block keeper
            if let Some(transaction_block_keeper) = self.transaction_block_keeper
                && storage.blocks.len() > transaction_block_keeper
            {
                let to_clear = block_number
                    .saturating_sub(transaction_block_keeper.try_into().unwrap_or(u64::MAX));
                storage.remove_block_transactions_by_number(to_clear)
            }

            // Live execution readers acquire the database read lock before cloning the block
            // environment, next-block fee state, and canonical head. Publish the complete snapshot
            // while retaining the database write lock so readers observe either the parent or newly
            // mined state. The database is always the outer lock, and no path holds a storage,
            // environment, or fee guard while waiting for it, so these acquisitions cannot form a
            // lock cycle.
            evm_env.block_env.difficulty = U256::ZERO;
            *self.evm_env.write() = evm_env;
            self.fees.set_next_block_fees(next_block_base_fee, next_block_blob_fees);
            let finalization =
                storage.tempo.as_ref().map_or(Ok(None), TempoChain::pending_finalization);
            drop(storage);
            drop(db_guard);
            if let Err(err) = finalization.and_then(|finalization| {
                finalization.map_or(Ok(()), |finalization| finalization.send())
            }) {
                warn!(target: "backend", %err, "failed to publish Tempo finalization");
            }

            self.time.mark_block_created();

            let timestamp = utc_from_secs(header.timestamp);

            node_info!("    Block Number: {}", block_number);
            node_info!("    Block Hash: {:?}", block_hash);
            if timestamp.year() > 9999 {
                // rf2822 panics with more than 4 digits
                node_info!("    Block Time: {:?}\n", timestamp.to_rfc3339());
            } else {
                node_info!("    Block Time: {:?}\n", timestamp.to_rfc2822());
            }

            let outcome =
                MinedBlockOutcome { block_number, included, stale, invalid, not_yet_valid };

            (outcome, header, block_hash)
        };
        // notify all listeners
        self.notify_on_new_block(header.into_inner(), block_hash);

        Ok(outcome)
    }

    /// Mines replacement blocks after the caller has rewound the chain for a reorg.
    ///
    /// The caller must hold the mining lock across ancestor selection, rollback, and this method.
    pub async fn reorg(
        &self,
        depth: u64,
        tx_pairs: HashMap<u64, Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>>,
        _mining_guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<(), BlockchainError> {
        // Create the new reorged chain, filling the blocks with transactions if supplied
        for i in 0..depth {
            let to_be_mined = tx_pairs.get(&i).cloned().unwrap_or_else(Vec::new);
            let outcome = self.do_mine_block_locked(to_be_mined, false).await?;
            node_info!(
                "    Mined reorg block number {}. With {} valid txs and with invalid {} txs",
                outcome.block_number,
                outcome.included.len(),
                outcome.invalid.len()
            );
        }

        Ok(())
    }

    /// Creates the pending block
    ///
    /// This will execute all transaction in the order they come but will not mine the block
    pub async fn pending_block(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
    ) -> Result<BlockInfo<N>, BlockchainError> {
        self.with_pending_block(pool_transactions, |_, block| block).await
    }

    /// Creates the pending block
    ///
    /// This will execute all transaction in the order they come but will not mine the block
    pub async fn with_pending_block<F, T>(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
        f: F,
    ) -> Result<T, BlockchainError>
    where
        F: FnOnce(Box<dyn MaybeFullDatabase + '_>, BlockInfo<N>) -> T,
    {
        let db = self.db.read().await;
        let evm_env = self.next_evm_env(&**db)?;

        let mut cache_db = CacheDB::new(&**db);

        let parent_hash = self.blockchain.storage.read().best_hash;
        let block_number = self.best_number().saturating_add(1);

        #[cfg(feature = "base")]
        let (pool_transactions, system_transaction_count) = {
            let mut transactions = self.base_system_transactions(
                &**db,
                block_number,
                evm_env.block_env.timestamp.saturating_to(),
                parent_hash,
            )?;
            let system_transaction_count = transactions.len();
            transactions.extend(pool_transactions);
            (transactions, system_transaction_count)
        };
        // Read the next block's beacon root override without consuming it, so the pending block
        // sees the root the next mined block will use.
        let parent_beacon_block_root = Some(
            self.cheats.next_block_overrides().parent_beacon_block_root.value.unwrap_or_default(),
        );

        let inspector_tx_config = self.inspector_tx_config();
        let gas_config = self.pool_tx_gas_config(&evm_env);

        let ExecutedBlock { pool: pool_result, result: block_result, state, .. } = self
            .execute_with_block_executor(
                &cache_db,
                &evm_env,
                parent_hash,
                self.hardfork(),
                parent_beacon_block_root,
                BlockExecutionKind::Complete,
                &pool_transactions,
                &gas_config,
                &inspector_tx_config,
                &|pool_tx, account| {
                    self.validate_pool_transaction_for(
                        &pool_tx.pending_transaction,
                        account,
                        &evm_env,
                    )
                },
            )?;

        #[cfg(feature = "base")]
        base::validate_system_transactions(
            &pool_transactions[..system_transaction_count],
            &pool_result,
        )?;

        cache_db.commit(&state);

        let state_root = cache_db.maybe_state_root().unwrap_or_default();
        let block_info = self.build_block_info(
            &evm_env,
            parent_hash,
            block_number,
            state_root,
            block_result,
            pool_result.txs,
            pool_result.tx_info,
            parent_beacon_block_root,
            None,
        );

        Ok(f(Box::new(cache_db), block_info))
    }

    /// Returns the account used to sponsor Tempo fee-payer requests handled by this node.
    ///
    /// Returns `None` on non-Tempo networks.
    pub async fn tempo_fee_payer(&self) -> Option<Address> {
        if !self.is_tempo() {
            return None;
        }
        self.node_config.read().await.tempo_fee_payer_address()
    }

    /// Returns the fee token an account pays with, as stored in the Tempo fee manager.
    ///
    /// Falls back to PathUSD when the account has no stored preference or the lookup fails.
    pub async fn tempo_user_fee_token(&self, account: Address) -> Result<Address, BlockchainError> {
        let calldata = IFeeManager::userTokensCall { user: account }.abi_encode();

        let request = WithOtherFields::new(TransactionRequest {
            from: Some(Address::ZERO),
            to: Some(TxKind::Call(TIP_FEE_MANAGER_ADDRESS)),
            input: calldata.into(),
            ..Default::default()
        });

        let outcome = self.call(request, FeeDetails::zero(), None, Default::default()).await?;

        let token = if outcome.exit == InstrStop::Return {
            IFeeManager::userTokensCall::abi_decode_returns(&outcome.output)
                .unwrap_or(Address::ZERO)
        } else {
            Address::ZERO
        };

        if token.is_zero() {
            return Ok(foundry_evm_core::tempo::PATH_USD_ADDRESS);
        }
        Ok(token)
    }

    /// Executes the [TransactionRequest] without writing to the DB
    ///
    /// # Errors
    ///
    /// Returns an error if the `block_number` is greater than the current height
    pub async fn call(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
        overrides: EvmOverrides,
    ) -> Result<CallOutcome, BlockchainError> {
        self.with_database_at_and_context(block_request, |state, mut block, monad_context| {
            let block_number = block.number;
            let outcome = {
                let mut cache_db = CacheDB::new(state);
                if let Some(state_overrides) = overrides.state {
                    apply_state_overrides(state_overrides, &mut cache_db)?;
                }
                if let Some(block_overrides) = overrides.block {
                    apply_block_overrides(*block_overrides, &mut cache_db, &mut block);
                }
                self.call_with_state_and_context(
                    &cache_db,
                    request,
                    fee_details,
                    block,
                    monad_context,
                )
            }?;
            trace!(target: "backend", "call return {:?} out: {:?} gas {} on block {}", outcome.exit, outcome.output, outcome.gas_used, block_number);
            Ok(outcome)
        })
        .await
    }

    pub async fn call_with_tracing(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
        opts: GethDebugTracingCallOptions,
    ) -> Result<GethTrace, BlockchainError> {
        let GethDebugTracingCallOptions {
            tracing_options,
            block_overrides,
            state_overrides,
            tx_index,
        } = opts;

        if let Some(tx_index) = tx_index {
            return self
                .call_with_tracing_at_tx_index(
                    request,
                    fee_details,
                    block_request,
                    tx_index,
                    tracing_options,
                    state_overrides,
                    block_overrides,
                )
                .await;
        }

        self.with_database_at_and_context(block_request, |state, block, monad_context| {
            let cache_db = CacheDB::new(&*state as &dyn DatabaseRef);
            self.trace_call_with_state(
                request,
                fee_details,
                block,
                cache_db,
                tracing_options,
                state_overrides,
                block_overrides,
                monad_context,
                None,
            )
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn call_with_tracing_at_tx_index(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
        tx_index: u64,
        tracing_options: GethDebugTracingOptions,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<BlockOverrides>,
    ) -> Result<GethTrace, BlockchainError> {
        let tx_index = usize::try_from(tx_index).map_err(|_| {
            BlockchainError::RpcError(RpcError::invalid_params(format!(
                "tx_index {tx_index} does not fit in usize"
            )))
        })?;
        let block_number = match block_request {
            Some(BlockRequest::Pending(_)) => {
                return Err(BlockchainError::RpcError(RpcError::invalid_params(
                    "tx_index is not supported for pending blocks".to_string(),
                )));
            }
            Some(BlockRequest::Number(number)) => number,
            None => self.best_number(),
        };
        let block_id = BlockId::number(block_number);

        if let Some(block) = self.get_block(block_id) {
            return self.mined_trace_call_at_tx_index(
                request,
                fee_details,
                &block,
                tx_index,
                tracing_options,
                state_overrides,
                block_overrides,
            );
        }

        if let Some(fork) = self.get_fork()
            && fork.predates_fork_inclusive(block_number)
        {
            let opts = GethDebugTracingCallOptions {
                tracing_options,
                state_overrides,
                block_overrides,
                tx_index: Some(tx_index as u64),
            };
            return Ok(fork.debug_trace_call(request, block_id, opts).await?);
        }

        Err(BlockchainError::BlockNotFound)
    }

    #[allow(clippy::too_many_arguments)]
    fn mined_trace_call_at_tx_index(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        block: &Block,
        tx_index: usize,
        tracing_options: GethDebugTracingOptions,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<BlockOverrides>,
    ) -> Result<GethTrace, BlockchainError> {
        let transaction_count = block.body.transactions.len();
        if tx_index >= transaction_count {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(format!(
                "tx_index {tx_index} out of bounds for block with {transaction_count} transactions"
            ))));
        }

        let trace = |parent_state: &StateDb| -> Result<GethTrace, BlockchainError> {
            let (mut cache_db, evm_env, hardfork) =
                self.prepare_block_replay_with_db(block, parent_state)?;
            self.replay_mined_transaction_prefix(
                &mut cache_db,
                &evm_env,
                hardfork,
                block,
                tx_index,
            )?;
            self.trace_call_with_state(
                request,
                fee_details,
                evm_env.block_env.clone(),
                cache_db,
                tracing_options,
                state_overrides,
                block_overrides,
                self.active_monad_context_before_mined_transaction(block, tx_index)?,
                Some((evm_env, hardfork)),
            )
        };

        let read_guard = self.states.upgradable_read();
        if let Some(state) = read_guard.get_state(&block.header.parent_hash) {
            trace(state)
        } else {
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            let state = write_guard
                .get_on_disk_state(&block.header.parent_hash)
                .ok_or(BlockchainError::HistoricalStateUnavailable(block.header.number()))?;
            trace(state)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn trace_call_with_state(
        &self,
        request: WithOtherFields<TransactionRequest>,
        fee_details: FeeDetails,
        mut block: BlockEnv,
        mut cache_db: CacheDB<&dyn DatabaseRef>,
        tracing_options: GethDebugTracingOptions,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<BlockOverrides>,
        mut monad_context: Option<MonadReplayContext>,
        historical_execution: Option<(EvmEnv, FoundryHardfork)>,
    ) -> Result<GethTrace, BlockchainError> {
        let GethDebugTracingOptions { config, tracer, tracer_config, .. } = tracing_options;
        let block_number = block.number;
        let base_evm_env = historical_execution.as_ref().map(|(evm_env, _)| evm_env);
        let hardfork = historical_execution
            .as_ref()
            .map(|(_, hardfork)| *hardfork)
            .unwrap_or_else(|| self.hardfork());

        if let Some(state_overrides) = state_overrides {
            apply_state_overrides(state_overrides, &mut cache_db)?;
        }
        if let Some(block_overrides) = block_overrides {
            apply_block_overrides(block_overrides, &mut cache_db, &mut block);
        }

        if let Some(tracer) = tracer {
            return match tracer {
                GethDebugTracerType::BuiltInTracer(tracer) => match tracer {
                    GethDebugBuiltInTracerType::CallTracer => {
                        let call_config = call_config_from_tracer_config(tracer_config)
                            .map_err(|e| RpcError::invalid_params(e.to_string()))?;

                        let mut inspector = self.build_inspector().with_tracing_config(
                            TracingInspectorConfig::from_geth_call_config(&call_config),
                        );

                        let PreparedCall { evm_env, tx, .. } = self.prepare_call_env_from_base(
                            &cache_db,
                            request,
                            fee_details,
                            block,
                            base_evm_env,
                        )?;
                        let executed = self.transact_call_with_inspector_ref_at_hardfork(
                            &cache_db,
                            &evm_env,
                            &mut inspector,
                            &tx,
                            monad_context.as_mut().map(next_monad_context),
                            hardfork,
                        )?;

                        inspector.print_logs();
                        if self.print_traces {
                            inspector.print_traces(self.call_trace_decoder());
                        }

                        let tracing_inspector = inspector.tracer.expect("tracer disappeared");

                        Ok(tracing_inspector
                            .into_geth_builder()
                            .geth_call_traces(call_config, executed.result.tx_gas_used())
                            .into())
                    }
                    GethDebugBuiltInTracerType::PreStateTracer => {
                        let pre_state_config = tracer_config
                            .into_pre_state_config()
                            .map_err(|e| RpcError::invalid_params(e.to_string()))?;

                        let mut inspector = TracingInspector::new(
                            TracingInspectorConfig::from_geth_prestate_config(&pre_state_config),
                        );

                        let PreparedCall { evm_env, tx, .. } = self.prepare_call_env_from_base(
                            &cache_db,
                            request,
                            fee_details,
                            block,
                            base_evm_env,
                        )?;
                        let executed = self.transact_call_with_inspector_ref_at_hardfork(
                            &cache_db,
                            &evm_env,
                            &mut inspector,
                            &tx,
                            monad_context.as_mut().map(next_monad_context),
                            hardfork,
                        )?;

                        Ok(geth_prestate_traces(
                            inspector,
                            executed,
                            &pre_state_config,
                            &cache_db,
                            evm_env.cfg_env.spec,
                        )?
                        .into())
                    }
                    GethDebugBuiltInTracerType::NoopTracer => Ok(NoopFrame::default().into()),
                    GethDebugBuiltInTracerType::FourByteTracer
                    | GethDebugBuiltInTracerType::MuxTracer
                    | GethDebugBuiltInTracerType::FlatCallTracer
                    | GethDebugBuiltInTracerType::Erc7562Tracer
                    | GethDebugBuiltInTracerType::StateGasTracer => {
                        Err(RpcError::invalid_params("unsupported tracer type").into())
                    }
                },
                #[cfg(not(feature = "js-tracer"))]
                GethDebugTracerType::JsTracer(_) => Err(js_tracer_unsupported()),
                #[cfg(feature = "js-tracer")]
                GethDebugTracerType::JsTracer(code) => {
                    let mut inspector = JsInspector::new(code, tracer_config.into_json())
                        .map_err(|err| RpcError::invalid_params(err.to_string()))?;
                    let PreparedCall { evm_env, tx, .. } = self.prepare_call_env_from_base(
                        &cache_db,
                        request,
                        fee_details,
                        block,
                        base_evm_env,
                    )?;
                    let executed = self.transact_call_with_inspector_ref_at_hardfork(
                        &cache_db,
                        &evm_env,
                        &mut inspector,
                        &tx,
                        monad_context.as_mut().map(next_monad_context),
                        hardfork,
                    )?;
                    with_evm_network!(self.evm_family(), E => {
                        js_trace_result::<E>(&mut inspector, executed, &tx, &evm_env, &cache_db)
                    })
                }
            };
        }

        // defaults to StructLog tracer used since no tracer is specified
        let mut inspector = self
            .build_inspector()
            .with_tracing_config(TracingInspectorConfig::from_geth_config(&config));

        let PreparedCall { evm_env, tx, .. } =
            self.prepare_call_env_from_base(&cache_db, request, fee_details, block, base_evm_env)?;
        let executed = self.transact_call_with_inspector_ref_at_hardfork(
            &cache_db,
            &evm_env,
            &mut inspector,
            &tx,
            monad_context.as_mut().map(next_monad_context),
            hardfork,
        )?;

        let tracing_inspector = inspector.tracer.expect("tracer disappeared");
        let result = executed.result;
        let exit_reason = result.stop;
        let gas_used = result.tx_gas_used();

        trace!(target: "backend", ?exit_reason, out = ?result.output, %gas_used, %block_number, "trace call");

        let res = tracing_inspector
            .into_geth_builder()
            .geth_traces_with_result_gas(&result, result.output.clone(), config)
            .into();

        Ok(res)
    }

    /// Helper function to execute a closure with the database at a specific block
    pub async fn with_database_at<F, T>(
        &self,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
        f: F,
    ) -> Result<T, BlockchainError>
    where
        F: FnOnce(Box<dyn MaybeFullDatabase + '_>, BlockEnv) -> T,
    {
        let block_number = match block_request {
            Some(BlockRequest::Pending(pool_transactions)) => {
                let result = self
                    .with_pending_block(pool_transactions, |state, block| {
                        let block = block.block;
                        f(state, self.block_env_from_header(&block.header))
                    })
                    .await?;
                return Ok(result);
            }
            Some(BlockRequest::Number(bn)) => Some(BlockNumber::Number(bn)),
            None => None,
        };
        let block_number = self.convert_block_number(block_number);
        let current_number = self.best_number();

        // Reject requests for future blocks that don't exist yet
        if block_number > current_number {
            return Err(BlockchainError::BlockOutOfRange(current_number, block_number));
        }

        // The head can advance while waiting for the database lock. Serve the live state only if
        // the requested block is still the head or no state history is kept. Otherwise hold the
        // lock so the requested block's state cannot be pruned before it is read below.
        let _head_guard = if block_number == current_number {
            let db = self.db.read().await;
            if self.best_number() == block_number
                || !self.prune_state_history_config.is_state_history_supported()
            {
                let block = self.evm_env.read().block_env.clone();
                return Ok(f(Box::new(&**db), block));
            }
            Some(db)
        } else {
            None
        };

        if let Some((block_hash, block)) = self
            .block_by_number(BlockNumber::Number(block_number))
            .await?
            .map(|block| (block.header.hash, block))
        {
            let read_guard = self.states.upgradable_read();
            if let Some(state_db) = read_guard.get_state(&block_hash) {
                return Ok(f(Box::new(state_db), self.block_env_from_header(&block.header)));
            }

            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            if let Some(state) = write_guard.get_on_disk_state(&block_hash) {
                return Ok(f(Box::new(state), self.block_env_from_header(&block.header)));
            }
        }

        warn!(target: "backend", "Not historic state found for block={}", block_number);
        Err(BlockchainError::BlockOutOfRange(self.best_number(), block_number))
    }

    /// Executes a closure with both state and network context at a specific block.
    pub(crate) async fn with_database_at_and_context<F, T>(
        &self,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
        f: F,
    ) -> Result<T, BlockchainError>
    where
        F: FnOnce(
            Box<dyn MaybeFullDatabase + '_>,
            BlockEnv,
            Option<MonadReplayContext>,
        ) -> Result<T, BlockchainError>,
    {
        let block_number = match block_request {
            Some(BlockRequest::Pending(pool_transactions)) => {
                return self
                    .with_pending_block(pool_transactions, |state, block_info| {
                        let context = self.active_monad_context_before_mined_transaction(
                            &block_info.block,
                            block_info.block.body.transactions.len(),
                        )?;
                        let block_env = self.block_env_from_header(&block_info.block.header);
                        f(state, block_env, context)
                    })
                    .await?;
            }
            Some(BlockRequest::Number(number)) => Some(BlockNumber::Number(number)),
            None => None,
        };
        let block_number = self.convert_block_number(block_number);
        let current_number = self.best_number();

        if block_number > current_number {
            return Err(BlockchainError::BlockOutOfRange(current_number, block_number));
        }

        #[cfg(feature = "monad")]
        let context = if self.is_monad() {
            Some(self.monad_context_for_child_of_block_number(block_number).await?)
        } else {
            None
        };
        #[cfg(not(feature = "monad"))]
        let context = None;

        // See `with_database_at`: re-check the head under the read guard.
        let _head_guard = if block_number == current_number {
            let db = self.db.read().await;
            if self.best_number() == block_number
                || !self.prune_state_history_config.is_state_history_supported()
            {
                let block = self.evm_env.read().block_env.clone();
                return f(Box::new(&**db), block, context);
            }
            Some(db)
        } else {
            None
        };

        if let Some((block_hash, block)) = self
            .block_by_number(BlockNumber::Number(block_number))
            .await?
            .map(|block| (block.header.hash, block))
        {
            let read_guard = self.states.upgradable_read();
            if let Some(state_db) = read_guard.get_state(&block_hash) {
                return f(Box::new(state_db), self.block_env_from_header(&block.header), context);
            }

            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            if let Some(state) = write_guard.get_on_disk_state(&block_hash) {
                return f(Box::new(state), self.block_env_from_header(&block.header), context);
            }
        }

        warn!(target: "backend", "Not historic state found for block={}", block_number);
        Err(BlockchainError::BlockOutOfRange(self.best_number(), block_number))
    }

    pub async fn storage_at(
        &self,
        address: Address,
        index: U256,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<B256, BlockchainError> {
        self.with_database_at(block_request, |db, _| {
            trace!(target: "backend", "get storage for {:?} at {:?}", address, index);
            let val = db.storage_ref(address, index)?;
            Ok(val.into())
        })
        .await?
    }

    pub async fn tempo_nonce(
        &self,
        caller: Address,
        nonce_key: U256,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<u64, BlockchainError> {
        self.with_database_at(block_request, |state, _| tempo_nonce(&state, caller, nonce_key))
            .await?
    }

    /// Returns storage values for multiple accounts and slots in a single call.
    pub async fn storage_values(
        &self,
        requests: HashMap<Address, Vec<B256>>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<HashMap<Address, Vec<B256>>, BlockchainError> {
        self.with_database_at(block_request, |db, _| {
            trace!(target: "backend", "get storage values for {} addresses", requests.len());
            let mut result: HashMap<Address, Vec<B256>> = HashMap::default();
            for (address, slots) in &requests {
                let mut values = Vec::with_capacity(slots.len());
                for slot in slots {
                    let val = db.storage_ref(*address, (*slot).into())?;
                    values.push(val.into());
                }
                result.insert(*address, values);
            }
            Ok(result)
        })
        .await?
    }

    /// Returns the code of the address
    ///
    /// If the code is not present and fork mode is enabled then this will try to fetch it from the
    /// forked client
    pub async fn get_code(
        &self,
        address: Address,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<Bytes, BlockchainError> {
        self.with_database_at(block_request, |db, _| self.get_code_with_state(&db, address)).await?
    }

    /// Returns the balance of the address
    ///
    /// If the requested number predates the fork then this will fetch it from the endpoint
    pub async fn get_balance(
        &self,
        address: Address,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<U256, BlockchainError> {
        self.with_database_at(block_request, |db, _| self.get_balance_with_state(db, address))
            .await?
    }

    pub async fn get_account_at_block(
        &self,
        address: Address,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<TrieAccount, BlockchainError> {
        self.with_database_at(block_request, |block_db, _| {
            let db = block_db.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
            let account = db.get(&address).cloned().unwrap_or_default();
            let storage_root = storage_root(&account.storage);
            let code_hash = account.info.code_hash;
            let balance = account.info.balance;
            let nonce = account.info.nonce;
            Ok(TrieAccount::new(nonce, balance, storage_root, code_hash))
        })
        .await?
    }

    pub async fn get_account_info_at_block(
        &self,
        address: Address,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<RpcAccountInfo, BlockchainError> {
        self.with_database_at(block_request, |db, _| {
            let account = db.basic_ref(address)?.unwrap_or_default();
            let code = self.get_code_with_state(&db, address)?;
            Ok(RpcAccountInfo { balance: account.balance, nonce: account.nonce, code })
        })
        .await?
    }

    /// Returns the nonce of the address
    ///
    /// If the requested number predates the fork then this will fetch it from the endpoint
    pub async fn get_nonce(
        &self,
        address: Address,
        block_request: BlockRequest<FoundryTxEnvelope>,
    ) -> Result<u64, BlockchainError> {
        if let BlockRequest::Pending(pool_transactions) = &block_request
            && let Some(value) = get_pool_transactions_nonce(pool_transactions, address)
        {
            return Ok(value);
        }
        let final_block_request = match block_request {
            BlockRequest::Pending(_) => BlockRequest::Number(self.best_number()),
            BlockRequest::Number(bn) => BlockRequest::Number(bn),
        };

        self.with_database_at(Some(final_block_request), |db, _| {
            trace!(target: "backend", "get nonce for {:?}", address);
            Ok(db.basic_ref(address)?.unwrap_or_default().nonce)
        })
        .await?
    }

    /// Traces the transaction with the js tracer
    #[cfg(feature = "js-tracer")]
    pub async fn trace_tx_with_js_tracer(
        &self,
        hash: B256,
        code: String,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace, BlockchainError> {
        // Boa is thread-local: construct, execute and drop the inspector before any await.
        let trace = {
            let context = self.blockchain.storage.read().transactions.get(&hash).map(|tx| {
                TransactionContext {
                    block_hash: Some(tx.block_hash),
                    tx_index: Some(tx.info.transaction_index as usize),
                    tx_hash: Some(hash),
                }
            });
            let inspector = JsInspector::with_transaction_context(
                code,
                opts.tracer_config.clone().into_json(),
                context.unwrap_or_default(),
            )
            .map_err(|err| RpcError::invalid_params(err.to_string()))?;
            self.replay_tx_with_inspector(
                hash,
                inspector,
                |executed, db, mut inspector, tx, env| {
                    with_evm_network!(self.evm_family(), E => {
                        js_trace_result::<E>(&mut inspector, executed, &tx, &env, db)
                    })
                },
            )
        };
        match trace {
            Ok(trace) => trace,
            Err(BlockchainError::TransactionNotFound) => {
                if let Some(fork) = self.get_fork() {
                    return Ok(fork.debug_trace_transaction(hash, opts).await?);
                }
                Err(BlockchainError::TransactionNotFound)
            }
            Err(err) => Err(err),
        }
    }

    /// Prove an account's existence or nonexistence in the state trie.
    ///
    /// Returns a merkle proof of the account's trie node, `account_key` == keccak(address)
    pub async fn prove_account_at(
        &self,
        address: Address,
        keys: Vec<B256>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<AccountProof, BlockchainError> {
        let block_number = block_request.as_ref().map(|r| r.block_number());
        if !matches!(block_request, Some(BlockRequest::Pending(_))) {
            let height = self.convert_block_number(block_number);
            if let Some(proof) = self.prove_tempo_account_at(address, &keys, height).await? {
                return Ok(proof);
            }
        }

        self.with_database_at(block_request, |block_db, _| {
            trace!(target: "backend", "get proof for {:?} at {:?}", address, block_number);
            let db = block_db.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
            Ok(prove_account(db, address, &keys))
        })
        .await?
    }
}

impl<N: Network> Backend<N>
where
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    /// Returns opcode gas usage for the given transaction.
    pub async fn trace_transaction_opcode_gas(
        &self,
        hash: B256,
    ) -> Result<Option<TransactionOpcodeGas>, BlockchainError> {
        match self.replay_tx_with_inspector(
            hash,
            OpcodeGasInspector::new(),
            move |_, _, inspector, _, _| TransactionOpcodeGas {
                transaction_hash: hash,
                opcode_gas: inspector.opcode_gas_iter().collect(),
            },
        ) {
            Ok(trace) => Ok(Some(trace)),
            Err(BlockchainError::TransactionNotFound) => {
                if let Some(fork) = self.get_fork() {
                    return Ok(fork.trace_transaction_opcode_gas(hash).await?);
                }

                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    /// Returns opcode gas usage for all transactions in the given block.
    pub async fn trace_block_opcode_gas(
        &self,
        block_id: BlockId,
    ) -> Result<Option<BlockOpcodeGas>, BlockchainError> {
        if let Some((block, block_hash)) = self.get_block_with_hash(block_id) {
            return self.mined_block_opcode_gas(&block, block_hash).map(Some);
        }

        if let Some(fork) = self.get_fork() {
            let number = self.ensure_block_number(Some(block_id)).await?;
            if fork.predates_fork_inclusive(number) {
                // Delegate the resolved block number so tags (`latest`/`pending`/`safe`/
                // `finalized`) are resolved against the fork's head instead of drifting with
                // the upstream chain. Hashes are forwarded unchanged.
                let resolved = match block_id {
                    BlockId::Hash(_) => block_id,
                    _ => BlockId::number(number),
                };
                return Ok(fork.trace_block_opcode_gas(resolved).await?);
            }
        }

        Err(BlockchainError::BlockNotFound)
    }

    fn mined_block_opcode_gas(
        &self,
        block: &Block,
        block_hash: B256,
    ) -> Result<BlockOpcodeGas, BlockchainError> {
        // Genesis has no parent state or protocol pre-execution to replay.
        if block.header.number() == self.genesis_number() {
            return Ok(BlockOpcodeGas {
                block_hash,
                block_number: block.header.number(),
                transactions: Vec::new(),
            });
        }

        let parent_hash = block.header.parent_hash;

        let trace = |parent_state: &StateDb| -> Result<Vec<TransactionOpcodeGas>, BlockchainError> {
            let (mut cache_db, evm_env, hardfork) =
                self.prepare_block_replay(block, parent_state)?;
            let mut transactions = Vec::with_capacity(block.body.transactions.len());
            let monad_context = self.active_monad_context_for_mined_block(block)?;

            for tx_envelope in &block.body.transactions {
                let mut inspector = OpcodeGasInspector::new();
                let pending_tx = self.pending_mined_transaction(tx_envelope.clone())?;
                let transaction_context =
                    monad_execution_context_at(monad_context.as_ref(), transactions.len());
                let executed = self.replay_envelope_with_inspector_ref_and_context(
                    &cache_db,
                    &evm_env,
                    &mut inspector,
                    &pending_tx,
                    EnvelopeExecution::replay(transaction_context, hardfork),
                )?;

                transactions.push(TransactionOpcodeGas {
                    transaction_hash: tx_envelope.hash(),
                    opcode_gas: inspector.opcode_gas_iter().collect(),
                });

                commit_pending(&mut cache_db, &executed.state);
            }

            Ok(transactions)
        };

        let read_guard = self.states.upgradable_read();
        let transactions = if let Some(state) = read_guard.get_state(&parent_hash) {
            trace(state)?
        } else {
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            let state = write_guard
                .get_on_disk_state(&parent_hash)
                .ok_or(BlockchainError::HistoricalStateUnavailable(block.header.number()))?;
            trace(state)?
        };

        Ok(BlockOpcodeGas { block_hash, block_number: block.header.number(), transactions })
    }

    /// Returns a best-effort execution witness for the given block, in the same format as reth's
    /// `debug_executionWitness`.
    ///
    /// Anvil does not track which state a block's execution actually touched, so this returns a
    /// witness for the entire parent state instead: the RLP encoding of every node of the parent
    /// state trie (including all storage tries), all contract codes, and the preimages of all
    /// account addresses and storage slots. This is a strict superset of the minimal witness, so
    /// stateless re-execution of the block against it works, but the witness size grows with the
    /// total state instead of the state accessed by the block.
    ///
    /// Limitations:
    /// - Not supported while forking: only remotely accessed accounts are known locally, and the
    ///   locally computed state roots do not match the remote chain's roots.
    /// - The parent block's state must still be available in the state history, i.e. it must not
    ///   have been discarded via `--prune-history`.
    /// - The genesis block has no witness since it has no parent state.
    /// - `headers` contains the ancestor headers within the 256 block `BLOCKHASH` window that are
    ///   known locally, which may be fewer than 256.
    pub async fn debug_execution_witness(
        &self,
        block: BlockNumber,
    ) -> Result<ExecutionWitness, BlockchainError> {
        let number = self.convert_block_number(Some(block));
        let best = self.best_number();
        if number > best {
            return Err(BlockchainError::BlockOutOfRange(best, number));
        }
        let Some(parent) = number.checked_sub(1) else {
            return Err(BlockchainError::Message(
                "genesis block has no parent state to build a witness from".to_string(),
            ));
        };

        let mut headers = Vec::new();
        for ancestor in (number.saturating_sub(BLOCKHASH_HISTORY)..number).rev() {
            let Some(block) = self.get_block(ancestor) else { break };
            headers.push(alloy_rlp::encode(&block.header).into());
        }

        self.with_database_at(Some(BlockRequest::Number(parent)), |state, _| {
            let Some(accounts) = state.maybe_full_db() else {
                return Err(BlockchainError::Message(
                    "debug_executionWitness is not supported while forking".to_string(),
                ));
            };

            let (_, nodes) = state_trie_witness(&accounts);
            let mut codes = Vec::new();
            let mut seen_codes = B256Set::default();
            let mut keys = Vec::new();
            for (address, account) in &accounts {
                keys.push(Bytes::copy_from_slice(address.as_slice()));
                for slot in account.storage.keys() {
                    keys.push(Bytes::from(slot.to_be_bytes::<32>()));
                }
                if account.info.code_hash != KECCAK256_EMPTY
                    && seen_codes.insert(account.info.code_hash)
                {
                    let code = match &account.info.code {
                        Some(code) => code.original_bytes(),
                        None => state.code_by_hash_ref(account.info.code_hash)?.original_bytes(),
                    };
                    codes.push(code);
                }
            }
            keys.sort_unstable();
            keys.dedup();

            Ok(ExecutionWitness { state: nodes, codes, keys, headers })
        })
        .await?
    }

    /// Returns account information after replaying a block through the transaction at `tx_index`.
    pub async fn debug_account_info_at(
        &self,
        block_id: BlockId,
        tx_index: Index,
        address: Address,
    ) -> Result<Option<RpcAccountInfo>, BlockchainError> {
        if let Some((block, _)) = self.get_block_with_hash(block_id) {
            return self.mined_debug_account_info_at(&block, tx_index, address).map(Some);
        }

        if let Some(fork) = self.get_fork() {
            let number = self.ensure_block_number(Some(block_id)).await?;
            if fork.predates_fork_inclusive(number) {
                // Delegate the resolved block number so tags (`latest`/`pending`/`safe`/
                // `finalized`) are resolved against the fork's head instead of drifting with
                // the upstream chain. Hashes are forwarded unchanged.
                let resolved = match block_id {
                    BlockId::Hash(_) => block_id,
                    _ => BlockId::number(number),
                };
                return Ok(fork.debug_account_info_at(resolved, tx_index, address).await?);
            }
        }

        Err(BlockchainError::BlockNotFound)
    }

    fn mined_debug_account_info_at(
        &self,
        block: &Block,
        tx_index: Index,
        address: Address,
    ) -> Result<RpcAccountInfo, BlockchainError> {
        let tx_index = tx_index.0;
        let transaction_count = block.body.transactions.len();
        if tx_index >= transaction_count {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(format!(
                "tx_index {tx_index} out of bounds for block with {transaction_count} transactions"
            ))));
        }

        let trace = |parent_state: &StateDb| -> Result<RpcAccountInfo, BlockchainError> {
            let (mut cache_db, evm_env, hardfork) =
                self.prepare_block_replay_with_db(block, parent_state)?;
            self.replay_mined_transaction_prefix(
                &mut cache_db,
                &evm_env,
                hardfork,
                block,
                tx_index + 1,
            )?;
            let account = cache_db.basic_ref(address)?.unwrap_or_default();
            let code = self.get_code_with_state(&cache_db, address)?;
            Ok(RpcAccountInfo { balance: account.balance, nonce: account.nonce, code })
        };

        let read_guard = self.states.upgradable_read();
        if let Some(state) = read_guard.get_state(&block.header.parent_hash) {
            trace(state)
        } else {
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            let state = write_guard
                .get_on_disk_state(&block.header.parent_hash)
                .ok_or(BlockchainError::HistoricalStateUnavailable(block.header.number()))?;
            trace(state)
        }
    }

    /// Rollback the chain to a common height.
    ///
    /// The state of the chain is rewound using `rewind` to the common block, including the db,
    /// storage, env, and next-block fees. Pending next-block overrides, including a base fee set
    /// with `anvil_setNextBlockBaseFeePerGas`, are discarded.
    pub async fn rollback(
        &self,
        common_block: Block,
        _mining_guard: &tokio::sync::MutexGuard<'_, ()>,
    ) -> Result<(), BlockchainError> {
        let hash = common_block.header.hash_slow();

        // Get the database at the common block
        let (common_state, replace_accounts) = {
            let return_state_or_throw_err =
                |db: Option<&StateDb>| -> Result<AddressMap<DbAccount>, BlockchainError> {
                    let state_db = db.ok_or(BlockchainError::DataUnavailable)?;
                    let db_full =
                        state_db.maybe_as_full_db().ok_or(BlockchainError::DataUnavailable)?;
                    Ok(db_full.clone())
                };

            let read_guard = self.states.upgradable_read();
            if let Some(db) = read_guard.get_state(&hash) {
                (return_state_or_throw_err(Some(db))?, true)
            } else {
                let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
                (return_state_or_throw_err(write_guard.get_on_disk_state(&hash))?, false)
            }
        };

        // Acquire the only awaited mutation lock before publishing any part of the rewind.
        let (rollback_gas_limit, rollback_base_fee) = if self
            .get_fork()
            .is_some_and(|fork| fork.block_number() == common_block.header.number())
        {
            let node_config = self.node_config.read().await;
            (
                self.get_block(common_block.header.number().saturating_add(1))
                    .map(|child| child.header.gas_limit())
                    .unwrap_or_else(|| node_config.gas_limit()),
                node_config.base_fee.unwrap_or_default(),
            )
        } else {
            (
                common_block.header.gas_limit(),
                common_block.header.base_fee_per_gas().unwrap_or_default(),
            )
        };
        let mut db = self.db.write().await;
        // Read the next-block fees while the retained block's child is still canonical.
        let (next_base_fee, next_blob_fees) = self.next_block_fees_after(&common_block.header);
        {
            // Collect the logs of the blocks that are about to be removed from the canonical
            // chain, while their transactions and receipts are still in storage
            let removed_logs = self.removed_logs_since(common_block.header.number());

            // Unwind the storage back to the common ancestor first
            let removed_blocks = {
                let mut storage = self.blockchain.storage.write();
                if let Some(tempo) = &mut storage.tempo {
                    tempo.unwind(&common_block)?;
                }
                #[cfg(test)]
                if let Some(hook) = self.rollback_unwind_hook.lock().as_ref() {
                    hook();
                }
                storage.unwind_to(common_block.header.number(), hash)
            };

            // Clean up in-memory and on-disk states for removed blocks
            let removed_hashes: Vec<_> =
                removed_blocks.iter().map(|b| b.header.hash_slow()).collect();
            self.states.write().remove_block_states(&removed_hashes);
            if let Some(states) = self.tempo_proof_states() {
                states.write().remove_block_states(&removed_hashes);
            }

            // Notify all log subscriptions and filters about the removed logs, so they receive
            // them again marked as removed, before any new chain notifications are emitted
            if !removed_logs.is_empty() {
                self.notify_on_removed_logs(removed_logs);
            }

            // Set environment back to common block
            let mut env = self.evm_env.write();
            env.block_env.number = self.evm_block_number(common_block.header.number());
            env.block_env.timestamp = U256::from(common_block.header.timestamp());
            env.block_env.gas_limit = rollback_gas_limit;
            env.block_env.difficulty = common_block.header.difficulty();
            env.block_env.prevrandao = common_block.header.mix_hash();
            env.block_env.basefee = rollback_base_fee;
            // Keep the blob environment set whenever Cancun is active, even for fork blocks whose
            // headers omit blob fields, because the EVM requires it.
            env.block_env.blob_excess_gas_and_price = self.fees.is_eip4844().then(|| {
                BlobExcessGasAndPrice::new(
                    common_block.header.excess_blob_gas().unwrap_or_default(),
                    self.fees.blob_params().update_fraction as u64,
                )
            });
            self.fees.set_next_block_fees(next_base_fee, next_blob_fees);

            self.time.reset(env.block_env.timestamp.saturating_to());
            // drop any pending next-block overrides so they do not leak into a block
            self.cheats.clear_next_block_overrides();
        }

        // Only collect the last 256 block hashes since that's all BLOCKHASH can access.
        let block_hashes: Vec<_> = {
            let storage = self.blockchain.storage.read();
            let min_block = common_block.header.number().saturating_sub(256);
            storage
                .hashes
                .iter()
                .filter(|(num, _)| **num >= min_block)
                .map(|(&num, &hash)| (num, hash))
                .collect()
        };

        db.clear();

        if replace_accounts && let Some(accounts) = db.maybe_as_full_db_mut() {
            *accounts = common_state;
        } else {
            // Insert account info before storage to prevent fork-mode RPC fetches after clear.
            for (address, acc) in common_state {
                db.insert_account(address, acc.info);
                for (key, value) in acc.storage {
                    db.set_storage_at(address, key.into(), value.into())?;
                }
            }
        }

        // Restore block hashes from blockchain storage (now unwound, contains only valid blocks).
        for (block_num, hash) in block_hashes {
            db.insert_block_hash(U256::from(block_num), hash);
        }

        Ok(())
    }

    /// Returns the traces for the given transaction
    pub async fn debug_trace_transaction(
        &self,
        hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace, BlockchainError> {
        #[cfg(feature = "js-tracer")]
        if let Some(tracer_type) = opts.tracer.as_ref()
            && tracer_type.is_js()
        {
            return self
                .trace_tx_with_js_tracer(hash, tracer_type.as_str().to_string(), opts.clone())
                .await;
        }

        if let Some(trace) = self.mined_geth_trace_transaction(hash, opts.clone()).await {
            return trace;
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.debug_trace_transaction(hash, opts).await?);
        }

        Err(BlockchainError::TransactionNotFound)
    }

    /// Returns geth-style traces for all transactions in an RLP-encoded block.
    pub async fn debug_trace_block(
        &self,
        rlp_block: Bytes,
        opts: GethDebugTracingOptions,
    ) -> Result<Vec<TraceResult>, BlockchainError> {
        let mut rlp = rlp_block.as_ref();
        let block = Block::<FoundryTxEnvelope>::decode(&mut rlp).map_err(|err| {
            BlockchainError::RpcError(RpcError::invalid_params(format!(
                "failed to decode block: {err}"
            )))
        })?;
        if !rlp.is_empty() {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(
                "failed to decode block: trailing bytes".to_string(),
            )));
        }

        self.debug_trace_block_by_hash(block.header.hash_slow(), opts).await
    }

    /// Returns geth-style traces for all transactions in a block by hash.
    pub async fn debug_trace_block_by_hash(
        &self,
        block_hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Result<Vec<TraceResult>, BlockchainError> {
        if let Some(block) = self.blockchain.get_block_by_hash(&block_hash) {
            return Ok(self.debug_trace_mined_block(&block, opts).await);
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.debug_trace_block_by_hash(block_hash, opts).await?);
        }

        Err(BlockchainError::BlockNotFound)
    }

    /// Returns geth-style traces for all transactions in a block by number.
    pub async fn debug_trace_block_by_number(
        &self,
        block_number: BlockNumber,
        opts: GethDebugTracingOptions,
    ) -> Result<Vec<TraceResult>, BlockchainError> {
        let number = self.convert_block_number(Some(block_number));

        if let Some(block) = self.get_block(BlockId::number(number)) {
            return Ok(self.debug_trace_mined_block(&block, opts).await);
        }

        if let Some(fork) = self.get_fork() {
            return Ok(fork.debug_trace_block_by_number(number, opts).await?);
        }

        Err(BlockchainError::BlockNotFound)
    }

    async fn debug_trace_mined_block(
        &self,
        block: &Block,
        opts: GethDebugTracingOptions,
    ) -> Vec<TraceResult> {
        if block.body.transactions.is_empty() {
            return Vec::new();
        }

        // A single transaction has no prefix replay to share.
        if block.body.transactions.len() > 1 {
            let traces = match &opts.tracer {
                Some(GethDebugTracerType::BuiltInTracer(
                    GethDebugBuiltInTracerType::CallTracer,
                )) => {
                    call_config_from_tracer_config(opts.tracer_config.clone()).ok().map(|config| {
                        self.replay_block_geth_traces(
                            block,
                            TracingInspectorConfig::from_geth_call_config(&config),
                            |inspector, gas_used, _| {
                                inspector.geth_builder().geth_call_traces(config, gas_used).into()
                            },
                        )
                    })
                }
                // Without steps tracing, struct logs come from the traces stored while mining.
                None if self.enable_steps_tracing => Some(self.replay_block_geth_traces(
                    block,
                    TracingInspectorConfig::from_geth_config(&opts.config),
                    |inspector, gas_used, output| {
                        inspector.geth_builder().geth_traces(gas_used, output, opts.config).into()
                    },
                )),
                _ => None,
            };
            if let Some(Ok(traces)) = traces {
                return traces;
            }
        }

        // Preserve per-transaction errors and fork lookup when sequential replay is unavailable.
        let mut traces = Vec::with_capacity(block.body.transactions.len());
        for tx in &block.body.transactions {
            let tx_hash = tx.hash();
            traces.push(match self.debug_trace_transaction(tx_hash, opts.clone()).await {
                Ok(result) => TraceResult::Success { result, tx_hash: Some(tx_hash) },
                Err(error) => {
                    TraceResult::Error { error: error.to_string(), tx_hash: Some(tx_hash) }
                }
            });
        }
        traces
    }

    /// Replays a local block once, keeping each transaction's committed state for the next call.
    fn replay_block_geth_traces(
        &self,
        block: &Block,
        tracing_config: TracingInspectorConfig,
        build_trace: impl Fn(&TracingInspector, u64, Bytes) -> GethTrace,
    ) -> Result<Vec<TraceResult>, BlockchainError> {
        let outcomes = {
            let storage = self.blockchain.storage.read();
            let block_hash = block.header.hash_slow();
            block
                .body
                .transactions
                .iter()
                .enumerate()
                .map(|(index, tx)| {
                    let hash = tx.hash();
                    let mined = storage
                        .transactions
                        .get(&hash)
                        .ok_or(BlockchainError::TransactionNotFound)?;
                    if mined.block_hash != block_hash
                        || mined.info.transaction_index as usize != index
                        || mined.info.transaction_hash != hash
                    {
                        return Err(BlockchainError::TransactionNotFound);
                    }
                    Ok((mined.info.gas_used, mined.info.out.clone().unwrap_or_default()))
                })
                .collect::<Result<Vec<_>, BlockchainError>>()?
        };

        let trace = |parent_state: &StateDb| -> Result<Vec<TraceResult>, BlockchainError> {
            let (mut cache_db, evm_env, hardfork) =
                self.prepare_block_replay(block, parent_state)?;
            let monad_context = self.active_monad_context_for_mined_block(block)?;
            let mut traces = Vec::with_capacity(block.body.transactions.len());
            for (index, (tx, (gas_used, output))) in
                block.body.transactions.iter().zip(&outcomes).enumerate()
            {
                let mut inspector = TracingInspector::new(tracing_config);
                let pending = self.pending_mined_transaction(tx.clone())?;
                let transaction_context = monad_execution_context_at(monad_context.as_ref(), index);
                let executed = self.replay_envelope_with_inspector_ref_and_context(
                    &cache_db,
                    &evm_env,
                    &mut inspector,
                    &pending,
                    EnvelopeExecution::replay(transaction_context, hardfork),
                )?;
                traces.push(TraceResult::Success {
                    result: build_trace(&inspector, *gas_used, output.clone()),
                    tx_hash: Some(tx.hash()),
                });
                commit_pending(&mut cache_db, &executed.state);
            }
            Ok(traces)
        };

        let read_guard = self.states.upgradable_read();
        if let Some(state) = read_guard.get_state(&block.header.parent_hash) {
            trace(state)
        } else {
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            let state = write_guard
                .get_on_disk_state(&block.header.parent_hash)
                .ok_or(BlockchainError::HistoricalStateUnavailable(block.header.number()))?;
            trace(state)
        }
    }

    fn geth_trace(
        &self,
        tx: &MinedTransaction<N>,
        opts: GethDebugTracingOptions,
    ) -> Result<GethTrace, BlockchainError> {
        let GethDebugTracingOptions { config, tracer, tracer_config, .. } = opts;

        if let Some(tracer) = tracer {
            match tracer {
                GethDebugTracerType::BuiltInTracer(tracer) => match tracer {
                    GethDebugBuiltInTracerType::FourByteTracer => {
                        let inspector = FourByteInspector::default();
                        let res = self.replay_tx_with_inspector(
                            tx.info.transaction_hash,
                            inspector,
                            |_, _, inspector, _, _| FourByteFrame::from(inspector).into(),
                        )?;
                        return Ok(res);
                    }
                    GethDebugBuiltInTracerType::CallTracer => {
                        return match call_config_from_tracer_config(tracer_config) {
                            Ok(call_config) => {
                                let inspector = TracingInspector::new(
                                    TracingInspectorConfig::from_geth_call_config(&call_config),
                                );
                                let frame = self.replay_tx_with_inspector(
                                    tx.info.transaction_hash,
                                    inspector,
                                    |_, _, inspector, _, _| {
                                        inspector
                                            .geth_builder()
                                            .geth_call_traces(call_config, tx.info.gas_used)
                                            .into()
                                    },
                                )?;
                                Ok(frame)
                            }
                            Err(e) => Err(RpcError::invalid_params(e.to_string()).into()),
                        };
                    }
                    GethDebugBuiltInTracerType::PreStateTracer => {
                        return match tracer_config.into_pre_state_config() {
                            Ok(pre_state_config) => {
                                let inspector = TracingInspector::new(
                                    TracingInspectorConfig::from_geth_prestate_config(
                                        &pre_state_config,
                                    ),
                                );
                                let frame = self.replay_tx_with_inspector(
                                    tx.info.transaction_hash,
                                    inspector,
                                    |executed, db, inspector, _, evm_env| {
                                        geth_prestate_traces(
                                            inspector,
                                            executed,
                                            &pre_state_config,
                                            db,
                                            evm_env.cfg_env.spec,
                                        )
                                    },
                                )??;
                                Ok(frame.into())
                            }
                            Err(e) => Err(RpcError::invalid_params(e.to_string()).into()),
                        };
                    }
                    GethDebugBuiltInTracerType::NoopTracer
                    | GethDebugBuiltInTracerType::MuxTracer
                    | GethDebugBuiltInTracerType::Erc7562Tracer
                    | GethDebugBuiltInTracerType::FlatCallTracer
                    | GethDebugBuiltInTracerType::StateGasTracer => {}
                },
                GethDebugTracerType::JsTracer(_code) => {}
            }

            return Ok(NoopFrame::default().into());
        }

        // default structlog tracer
        let return_value = tx.info.out.clone().unwrap_or_default();

        // Steps are not recorded when mining because they would be kept for every transaction, so
        // replay the transaction to record only what this request asks for.
        if self.enable_steps_tracing {
            let inspector =
                TracingInspector::new(TracingInspectorConfig::from_geth_config(&config));
            return self.replay_tx_with_inspector(
                tx.info.transaction_hash,
                inspector,
                |_, _, inspector, _, _| {
                    inspector
                        .geth_builder()
                        .geth_traces(tx.info.gas_used, return_value, config)
                        .into()
                },
            );
        }

        Ok(GethTraceBuilder::new_borrowed(&tx.info.traces)
            .geth_traces(tx.info.gas_used, return_value, config)
            .into())
    }

    async fn mined_geth_trace_transaction(
        &self,
        hash: B256,
        opts: GethDebugTracingOptions,
    ) -> Option<Result<GethTrace, BlockchainError>> {
        self.blockchain.storage.read().transactions.get(&hash).map(|tx| self.geth_trace(tx, opts))
    }

    pub async fn transaction_receipt(
        &self,
        hash: B256,
    ) -> Result<Option<FoundryTxReceipt>, BlockchainError> {
        if let Some(receipt) = self.mined_transaction_receipt(hash) {
            return Ok(Some(receipt.inner));
        }

        if let Some(fork) = self.get_fork() {
            let receipt = fork.transaction_receipt(hash).await?;
            let number = self.convert_block_number(
                receipt.clone().and_then(|r| r.block_number()).map(BlockNumber::from),
            );

            if fork.predates_fork_inclusive(number) {
                return Ok(receipt);
            }
        }

        Ok(None)
    }

    /// Returns all transaction receipts of the block
    pub fn mined_block_receipts(&self, id: impl Into<BlockId>) -> Option<Vec<FoundryTxReceipt>> {
        let storage = self.blockchain.storage.read();
        let hash = match id.into() {
            BlockId::Hash(hash) => hash.block_hash,
            BlockId::Number(number) => storage.hash(number, self.slots_in_an_epoch)?,
        };
        let block = storage.blocks.get(&hash)?.clone();

        if block.body.transactions.iter().enumerate().any(|(index, transaction)| {
            storage.transactions.get(&transaction.hash()).is_none_or(|transaction| {
                transaction.block_hash != hash
                    || transaction.info.transaction_index as usize != index
            })
        }) {
            drop(storage);
            return block
                .body
                .transactions
                .into_iter()
                .map(|transaction| {
                    self.mined_transaction_receipt(transaction.hash()).map(|receipt| receipt.inner)
                })
                .collect();
        }

        let mut receipts = Vec::with_capacity(block.body.transactions.len());
        let mut next_log_index = 0;

        for block_transaction in &block.body.transactions {
            let transaction = storage.transactions.get(&block_transaction.hash())?;
            let log_count = transaction.receipt.logs().len();
            let receipt = self.build_mined_transaction_receipt(
                &transaction.info,
                transaction.receipt.clone(),
                transaction.block_hash,
                &block,
                next_log_index,
            );
            receipts.push(receipt.inner);
            next_log_index += log_count;
        }

        Some(receipts)
    }

    /// Returns the transaction receipt for the given hash
    pub(crate) fn mined_transaction_receipt(
        &self,
        hash: B256,
    ) -> Option<MinedTransactionReceipt<FoundryNetwork>> {
        let storage = self.blockchain.storage.read();
        let transaction = storage.transactions.get(&hash)?;

        let index = transaction.info.transaction_index as usize;
        let block = storage.blocks.get(&transaction.block_hash)?;
        let mut next_log_index = 0;
        for block_transaction in &block.body.transactions[..index] {
            next_log_index +=
                storage.transactions.get(&block_transaction.hash())?.receipt.logs().len();
        }

        Some(self.build_mined_transaction_receipt(
            &transaction.info,
            transaction.receipt.clone(),
            transaction.block_hash,
            block,
            next_log_index,
        ))
    }

    fn build_mined_transaction_receipt(
        &self,
        info: &TransactionInfo,
        tx_receipt: FoundryReceiptEnvelope,
        block_hash: B256,
        block: &Block,
        next_log_index: usize,
    ) -> MinedTransactionReceipt<FoundryNetwork> {
        let transaction = block.body.transactions[info.transaction_index as usize].clone();

        // Cancun specific
        let blob_gas_used = transaction.blob_gas_used();
        let blob_gas_price = blob_gas_used.and_then(|_| block.header.blob_fee(self.blob_params()));

        let effective_gas_price = transaction.effective_gas_price(block.header.base_fee_per_gas());

        #[cfg(feature = "base")]
        let eip8130_phase_statuses = tx_receipt.eip8130_phase_statuses().to_vec();
        let tx_receipt = tx_receipt.convert_logs_rpc(
            BlockNumHash::new(block.header.number(), block_hash),
            block.header.timestamp(),
            info.transaction_hash,
            info.transaction_index,
            next_log_index,
        );

        let receipt = TransactionReceipt {
            inner: tx_receipt,
            transaction_hash: info.transaction_hash,
            transaction_index: Some(info.transaction_index),
            block_number: Some(block.header.number()),
            gas_used: info.gas_used,
            contract_address: info.contract_address,
            effective_gas_price,
            block_hash: Some(block_hash),
            from: info.from,
            to: info.to,
            blob_gas_price,
            blob_gas_used,
        };

        // Include timestamp in receipt to avoid extra block lookups (e.g., in Otterscan API)
        let mut inner = FoundryTxReceipt::with_timestamp(receipt, block.header.timestamp());
        if let FoundryTxEnvelope::Celo(tx) = &*transaction {
            inner
                .0
                .other
                .insert("feeCurrency".into(), serde_json::to_value(tx.tx().fee_currency).unwrap());
        }
        if self.is_tempo() {
            let fee_payer = match &*transaction {
                FoundryTxEnvelope::Tempo(tx) => match tx.tx().recover_fee_payer(info.from) {
                    Ok(fee_payer) => fee_payer,
                    Err(error) => {
                        warn!(
                            target: "backend",
                            %error,
                            tx_hash = ?info.transaction_hash,
                            "failed to recover Tempo fee payer for mined receipt"
                        );
                        info.from
                    }
                },
                _ => info.from,
            };
            inner = inner.with_fee_payer(fee_payer);

            // Match Tempo's receipt conversion: the final log of every non-free
            // transaction is the fee token transfer to TIPFeeManager.
            if inner.effective_gas_price() > 0
                && inner.gas_used() > 0
                && let Some(fee_token) = inner.0.inner.logs().last().map(|log| log.address())
            {
                inner = inner.with_fee_token(fee_token);
            }
        }
        #[cfg(feature = "base")]
        if self.is_base()
            && let FoundryTxEnvelope::Eip8130(tx) = transaction.as_ref()
        {
            let fields = Eip8130ReceiptFields {
                payer: Some(tx.tx().payer.unwrap_or(info.from)),
                phase_statuses: eip8130_phase_statuses,
                metadata: (!tx.tx().metadata.is_empty()).then(|| tx.tx().metadata.clone()),
            };
            inner
                .0
                .other
                .extend(OtherFields::try_from(fields).expect("EIP-8130 receipt fields serialize"));
        }
        MinedTransactionReceipt { inner, out: info.out.clone() }
    }

    /// Executes the pending block and returns its transaction receipts.
    pub async fn pending_block_receipts(
        &self,
        pool_transactions: Vec<Arc<PoolTransaction<FoundryTxEnvelope>>>,
    ) -> Result<Vec<FoundryTxReceipt>, BlockchainError> {
        let BlockInfo { block, transactions, receipts } =
            self.pending_block(pool_transactions).await?;
        let block_hash = block.header.hash_slow();
        let mut pending_receipts = Vec::with_capacity(receipts.len());
        let mut next_log_index = 0;

        for (info, receipt) in transactions.iter().zip(receipts) {
            let log_count = receipt.logs().len();
            let receipt = self.build_mined_transaction_receipt(
                info,
                receipt,
                block_hash,
                &block,
                next_log_index,
            );
            pending_receipts.push(receipt.inner);
            next_log_index += log_count;
        }

        Ok(pending_receipts)
    }

    /// Returns the blocks receipts for the given number
    pub async fn block_receipts(
        &self,
        number: BlockId,
    ) -> Result<Option<Vec<FoundryTxReceipt>>, BlockchainError> {
        if let Some(receipts) = self.mined_block_receipts(number) {
            return Ok(Some(receipts));
        }

        if let Some(fork) = self.get_fork() {
            let number = match self.ensure_block_number(Some(number)).await {
                Err(_) => return Ok(None),
                Ok(n) => n,
            };

            if fork.predates_fork_inclusive(number) {
                let receipts = fork.block_receipts(number).await?;

                return Ok(receipts);
            }
        }

        Ok(None)
    }
}

impl<N: Network<ReceiptEnvelope = FoundryReceiptEnvelope>> Backend<N> {
    /// Get the current state.
    pub async fn serialized_state(
        &self,
        preserve_historical_states: bool,
    ) -> Result<SerializableState, BlockchainError> {
        // Keep account state and head metadata coherent across mining and state replacement.
        let _mining_guard = self.mining.lock().await;
        let at = self.evm_env.read().block_env.clone();
        #[cfg(feature = "monad")]
        let mut monad_block_participants = BTreeMap::new();
        #[cfg(feature = "monad")]
        let mut monad_block_replay_profiles = BTreeMap::new();
        let (best_number, blocks, transactions) = {
            let storage = self.blockchain.storage.read();
            #[cfg(feature = "monad")]
            if self.is_monad() {
                monad_block_participants = storage
                    .monad_block_participants
                    .iter()
                    .filter(|(hash, _)| storage.blocks.contains_key(*hash))
                    .map(|(hash, participants)| (*hash, participants.iter().copied().collect()))
                    .collect();
                monad_block_replay_profiles = storage
                    .monad_block_replay_profiles
                    .iter()
                    .filter(|(hash, _)| storage.blocks.contains_key(*hash))
                    .map(|(hash, profile)| (*hash, *profile))
                    .collect();
            }
            (storage.best_number, storage.serialized_blocks(), storage.serialized_transactions())
        };
        let historical_states =
            preserve_historical_states.then(|| self.states.write().serialized_states());

        let state = self
            .db
            .read()
            .await
            .dump_state(at, best_number, blocks, transactions, historical_states)?
            .ok_or_else(|| {
                BlockchainError::RpcError(RpcError::invalid_params(
                    "Dumping state not supported with the current configuration",
                ))
            })?;
        #[cfg(feature = "monad")]
        let state = {
            let mut state = state;
            state.monad_block_participants = monad_block_participants;
            state.monad_block_replay_profiles = monad_block_replay_profiles;
            state
        };
        Ok(state)
    }

    /// Write all chain data to serialized bytes buffer
    pub async fn dump_state(
        &self,
        preserve_historical_states: bool,
    ) -> Result<Bytes, BlockchainError> {
        let state = self.serialized_state(preserve_historical_states).await?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&serde_json::to_vec(&state).unwrap_or_default())
            .map_err(|_| BlockchainError::DataUnavailable)?;
        Ok(encoder.finish().unwrap_or_default().into())
    }

    /// Apply [SerializableState] data to the backend storage.
    pub async fn load_state(&self, mut state: SerializableState) -> Result<bool, BlockchainError> {
        let _mining_guard = self.mining.lock().await;
        let mut block_env = state.block.take();
        let mut fork_block_boundary = None;
        let mut selected_head = None;
        let mut selected_header = None;
        let mut checkpoint = None;
        let fork_head = self.get_fork().map(|f| (f.block_number(), f.block_hash(), f.timestamp()));
        if let Some(block) = &mut block_env {
            if self.is_tempo() && self.is_fork() && block.beneficiary.is_zero() {
                block.beneficiary = TIP_FEE_MANAGER_ADDRESS;
            }
            // Set the current best block number.
            // Defaults to block number for compatibility with existing state files.
            let best_number = state.best_block_number.unwrap_or(block.number.saturating_to());
            let (selected_best_number, selected_best_hash) = if let Some((number, hash, _)) =
                fork_head
            {
                trace!(target: "backend", state_block_number=?best_number, fork_block_number=?number);
                // If the state.block_number is greater than the fork block number, set best number
                // to the state block number.
                // Ref: https://github.com/foundry-rs/foundry/issues/9539
                if best_number > number {
                    (best_number, None)
                } else {
                    // If loading state file on a fork, set best number to the fork block number.
                    // Ref: https://github.com/foundry-rs/foundry/pull/9215#issue-2618681838
                    fork_block_boundary = Some(number);
                    (number, Some(hash))
                }
            } else {
                (best_number, None)
            };

            let best_hash = if let Some(hash) = selected_best_hash {
                selected_header = state
                    .blocks
                    .iter()
                    .rev()
                    .find(|block| block.header.hash_slow() == hash)
                    .map(|block| block.header.clone());
                hash
            } else if state.blocks.is_empty() {
                let spec_id = self.spec_id();
                let is_cancun = spec_id >= SpecId::CANCUN;
                let parent_hash = selected_best_number
                    .checked_sub(1)
                    .and_then(|number| self.blockchain.storage.read().hashes.get(&number).copied())
                    .unwrap_or_default();
                let header = Header {
                    parent_hash,
                    beneficiary: block.beneficiary,
                    difficulty: block.difficulty,
                    number: selected_best_number,
                    gas_limit: block.gas_limit,
                    timestamp: block.timestamp.saturating_to(),
                    mix_hash: block.prevrandao.unwrap_or_default(),
                    base_fee_per_gas: (spec_id >= SpecId::LONDON).then_some(block.basefee),
                    parent_beacon_block_root: is_cancun.then_some(Default::default()),
                    blob_gas_used: is_cancun.then_some(0),
                    excess_blob_gas: if is_cancun { block.blob_excess_gas() } else { None },
                    withdrawals_root: (spec_id >= SpecId::SHANGHAI).then_some(EMPTY_WITHDRAWALS),
                    requests_hash: (spec_id >= SpecId::PRAGUE).then_some(EMPTY_REQUESTS_HASH),
                    ..Default::default()
                };
                let header = foundry_header(&self.networks, header);
                let best_hash = header.hash_slow();
                selected_header = Some(header.clone());
                checkpoint = Some(create_block(
                    header,
                    Vec::<MaybeImpersonatedTransaction<FoundryTxEnvelope>>::new(),
                ));
                warn!(
                    target: "backend",
                    block_number = selected_best_number,
                    "state dump has no block history; created a synthetic checkpoint block"
                );
                best_hash
            } else if let Some(header) = state
                .blocks
                .iter()
                .rev()
                .find(|block| block.header.number() == selected_best_number)
                .map(|block| block.header.clone())
            {
                let best_hash = header.hash_slow();
                selected_header = Some(header);
                best_hash
            } else {
                return Err(BlockchainError::RpcError(RpcError::internal_error_with(format!(
                    "Best hash not found for best number {selected_best_number}",
                ))));
            };

            selected_head = Some((selected_best_number, best_hash));
        }

        // Stage the complete chain update first. Besides keeping blocks and transactions atomic for
        // concurrent readers, this ensures validation failures cannot leave a partially loaded
        // chain behind.
        let blocks = std::mem::take(&mut state.blocks);
        let transactions = std::mem::take(&mut state.transactions);
        let mut storage = self.blockchain.storage.read().clone();
        storage.load_blocks(blocks, fork_block_boundary);
        if !self.is_celo()
            && storage.blocks.values().any(|block| {
                block
                    .body
                    .transactions
                    .iter()
                    .any(|tx| matches!(tx.as_ref(), FoundryTxEnvelope::Celo(_)))
            })
        {
            self.ensure_cip64_active()?;
        }
        storage.load_transactions(transactions, fork_block_boundary);
        if let Some(checkpoint) = checkpoint {
            storage.insert_block(checkpoint);
        }
        if let Some((number, hash)) = selected_head {
            storage.hashes.insert(number, hash);
            storage.best_number = number;
            storage.best_hash = hash;
            storage.hashes.retain(|block_number, _| *block_number <= number);
        }

        #[cfg(feature = "monad")]
        if self.is_monad() {
            for (hash, profile) in &state.monad_block_replay_profiles {
                if storage.blocks.contains_key(hash) {
                    storage.monad_block_replay_profiles.insert(*hash, *profile);
                }
            }
            for (hash, participants) in &state.monad_block_participants {
                if storage.blocks.contains_key(hash) {
                    storage
                        .monad_block_participants
                        .insert(*hash, participants.iter().copied().collect());
                }
            }
            self.rebuild_monad_block_participant_cache(&mut storage)?;
            // Reject state that cannot supply the ancestor metadata required by the next block
            // before changing the live chain, EVM environment, or database.
            self.monad_context_for_child_of_in_storage(&storage, storage.best_hash)?;
        }

        // Re-anchor block time to the canonical head selected above so the next blocks continue
        // its timeline: the saved one when the loaded head stays canonical, the fork's when the
        // state file is at or below the fork block. Resolve the timestamp from staged storage so a
        // later validation or database failure cannot modify the live clock.
        let canonical_timestamp = match fork_head {
            Some((_, fork_hash, fork_timestamp)) if storage.best_hash == fork_hash => {
                Some(fork_timestamp)
            }
            _ => storage.blocks.get(&storage.best_hash).map(|block| block.header.timestamp),
        };

        if let Some(block) = block_env.as_mut() {
            // Keep NUMBER aligned with the canonical local head chosen above. Arbitrum state dumps
            // can intentionally keep BlockEnv.number distinct from the best L2 block number.
            if !is_arbitrum(self.protocol_chain_id())
                && let Some((number, _)) = selected_head
            {
                block.number = U256::from(number);
            }
        }

        let next_fees = selected_header.as_ref().map(|header| {
            let parent_fees = self.fees.get_parent_header_fees(header);
            let blob_params = self.fees.blob_params();
            let next_block_excess_blob_gas = self.networks.next_block_blob_excess_gas(
                blob_params,
                header.excess_blob_gas().unwrap_or_default(),
                header.blob_gas_used().unwrap_or_default(),
                header.base_fee_per_gas().unwrap_or_default(),
            );
            let blob_excess_gas_and_price = BlobExcessGasAndPrice::new(
                next_block_excess_blob_gas,
                blob_params.update_fraction as u64,
            );
            (parent_fees, blob_excess_gas_and_price)
        });

        let historical_states = state.historical_states.take();
        let mut db = self.db.write().await;
        let db_snapshot = db.snapshot_state();
        let load_result = (|| -> Result<(), BlockchainError> {
            if !db.load_state(state)? {
                return Err(RpcError::invalid_params(
                    "Loading state not supported with the current configuration",
                )
                .into());
            }
            // A pre-Denim dump has no BaseTime deployment, but Denim blocks still send its
            // deposit.
            #[cfg(feature = "base")]
            if self.is_base() && self.base_upgrade() >= BaseUpgrade::Denim {
                base::ensure_base_time_predeploy(&mut **db, false)?;
            }
            Ok(())
        })();
        if let Err(err) = load_result {
            db.revert_state(db_snapshot, RevertStateSnapshotAction::RevertRemove);
            return Err(err);
        }
        db.delete_state_snapshot(db_snapshot);

        // Backfill the EVM-level block hash cache from the freshly loaded blocks so that the
        // BLOCKHASH opcode stays consistent after loading state. Reuses the hashes already
        // computed by `load_blocks` above. Only collect the last 256 blocks since that's all
        // BLOCKHASH can access.
        let block_hashes = {
            let min_block = storage.best_number.saturating_sub(256);
            storage
                .hashes
                .iter()
                .filter(|(num, _)| (min_block..=storage.best_number).contains(*num))
                .map(|(&num, &hash)| (U256::from(num), hash))
                .collect()
        };

        let reanchor_tempo = {
            // The database guard keeps chain identity stable until publication. Compare under a
            // read guard so existing chain readers need not wait for a large history comparison.
            let live_storage = self.blockchain.storage.read();
            live_storage.tempo.is_some()
                && (storage.best_number != live_storage.best_number
                    || storage.best_hash != live_storage.best_hash
                    || storage.genesis_number != live_storage.genesis_number
                    || storage.genesis_hash != live_storage.genesis_hash
                    || storage.hashes != live_storage.hashes)
        };
        {
            let mut live_storage = self.blockchain.storage.write();
            // Keep the live consensus, including snapshots or rotations created since staging.
            storage.tempo = live_storage.tempo.take();
            if reanchor_tempo && let Some(tempo) = &mut storage.tempo {
                tempo.reanchor(storage.best_number);
            }
            *live_storage = storage;
        }
        if let Some(timestamp) = canonical_timestamp {
            self.time.reset(timestamp);
        }
        if let Some(block_env) = block_env {
            self.evm_env.write().block_env = block_env;
        }
        if let Some((parent_fees, blob_excess_gas_and_price)) = next_fees {
            #[cfg(feature = "optimism")]
            if self.is_optimism() {
                self.fees.set_optimism_base_fee_rules(&parent_fees.extra_data);
            }
            self.fees.set_base_fee(parent_fees.base_fee);
            self.fees.set_blob_excess_gas_and_price(blob_excess_gas_and_price);
        }

        db.set_block_hashes(block_hashes);

        if let Some(historical_states) = historical_states {
            let mut states = self.states.write();
            for (hash, state_snapshot) in historical_states {
                states.insert(hash, db.historical_state(state_snapshot));
            }
        }

        Ok(true)
    }

    /// Deserialize and add all chain data to the backend storage
    pub async fn load_state_bytes(&self, buf: Bytes) -> Result<bool, BlockchainError> {
        let orig_buf = &buf.0[..];
        let mut decoder = GzDecoder::new(orig_buf);
        let mut decoded_data = Vec::new();

        let state: SerializableState = serde_json::from_slice(if decoder.header().is_some() {
            decoder
                .read_to_end(decoded_data.as_mut())
                .map_err(|_| BlockchainError::FailedToDecodeStateDump)?;
            &decoded_data
        } else {
            &buf.0
        })
        .map_err(|_| BlockchainError::FailedToDecodeStateDump)?;

        self.load_state(state).await
    }
}

impl Backend<FoundryNetwork> {
    /// Simulates a bundle of signed transactions and returns Flashbots-compatible results.
    pub async fn call_bundle(
        &self,
        bundle: EthCallBundle,
        transactions: Vec<PendingTransaction<FoundryTxEnvelope>>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
    ) -> Result<EthCallBundleResponse, BlockchainError> {
        let EthCallBundle {
            block_number,
            coinbase,
            timestamp,
            gas_limit,
            difficulty,
            base_fee,
            ..
        } = bundle;

        let blob_gas_used = transactions
            .iter()
            .filter_map(|transaction| transaction.transaction.blob_gas_used())
            .sum::<u64>();
        let max_blob_gas = self.blob_params().max_blob_gas_per_block();
        if blob_gas_used > max_blob_gas {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(format!(
                "blob gas usage exceeds the limit of {max_blob_gas} gas per block."
            ))));
        }

        self.with_database_at_and_context(
            block_request,
            |state, mut block_env, mut monad_context| {
                let state_block_number = block_env.number.to::<u64>();
                block_env.number = U256::from(block_number);
                block_env.timestamp = timestamp
                    .map(U256::from)
                    .unwrap_or_else(|| block_env.timestamp.saturating_add(U256::from(12)));
                if let Some(coinbase) = coinbase {
                    block_env.beneficiary = coinbase;
                }
                if let Some(gas_limit) = gas_limit {
                    block_env.gas_limit = gas_limit;
                }
                if let Some(difficulty) = difficulty {
                    block_env.difficulty = difficulty;
                }
                if let Some(base_fee) = base_fee {
                    block_env.basefee = base_fee.try_into().unwrap_or(u64::MAX);
                }

                let mut evm_env = self.evm_env.read().clone();
                evm_env.block_env = block_env;
                let coinbase = evm_env.block_env.beneficiary;
                let base_fee = evm_env.block_env.basefee;
                let mut cache_db = CacheDB::new(state);
                let initial_coinbase = cache_db
                    .basic_ref(coinbase)?
                    .map(|account| account.balance)
                    .unwrap_or_default();
                let mut coinbase_balance_before_tx = initial_coinbase;
                let mut coinbase_balance_after_tx = initial_coinbase;
                let mut total_gas_used = 0u64;
                let mut total_gas_fees = U256::ZERO;
                let mut bundle_hash = alloy_primitives::Keccak256::new();
                let mut results = Vec::with_capacity(transactions.len());

                for transaction in transactions {
                    let sender = *transaction.sender();
                    let tx = transaction.transaction.as_ref();
                    let tx_hash = tx.hash();
                    bundle_hash.update(tx_hash);

                    let mut inspector = self.build_inspector();
                    let Executed { result, state } = self
                        .transact_envelope_with_inspector_ref_and_context(
                            &cache_db,
                            &evm_env,
                            &mut inspector,
                            &transaction,
                            monad_context.as_mut().map(next_monad_context),
                        )?;

                    let gas_price = tx.effective_tip_per_gas(base_fee).unwrap_or_default();
                    let gas_used = result.tx_gas_used();
                    let gas_fees = U256::from(gas_used) * U256::from(gas_price);
                    total_gas_used += gas_used;
                    total_gas_fees += gas_fees;

                    coinbase_balance_after_tx = state
                        .account_info(&coinbase)
                        .map(|account| account.balance)
                        .unwrap_or(coinbase_balance_before_tx);
                    let coinbase_diff =
                        coinbase_balance_after_tx.saturating_sub(coinbase_balance_before_tx);
                    let eth_sent_to_coinbase = coinbase_diff.saturating_sub(gas_fees);
                    coinbase_balance_before_tx = coinbase_balance_after_tx;

                    let output = result.output.clone();
                    let (value, revert) =
                        if result.status { (Some(output), None) } else { (None, Some(output)) };

                    results.push(EthCallBundleTransactionResult {
                        coinbase_diff,
                        eth_sent_to_coinbase,
                        from_address: sender,
                        gas_fees,
                        gas_price: U256::from(gas_price),
                        gas_used,
                        to_address: tx.to(),
                        tx_hash,
                        value,
                        revert,
                    });
                    commit_pending(&mut cache_db, &state);
                }

                let coinbase_diff = coinbase_balance_after_tx.saturating_sub(initial_coinbase);
                let eth_sent_to_coinbase = coinbase_diff.saturating_sub(total_gas_fees);
                let bundle_gas_price =
                    coinbase_diff.checked_div(U256::from(total_gas_used)).unwrap_or_default();

                Ok(EthCallBundleResponse {
                    bundle_hash: bundle_hash.finalize(),
                    bundle_gas_price,
                    coinbase_diff,
                    eth_sent_to_coinbase,
                    gas_fees: total_gas_fees,
                    results,
                    state_block_number,
                    total_gas_used,
                })
            },
        )
        .await
    }

    /// Executes bundles of call requests and returns each call output.
    pub async fn call_many(
        &self,
        bundles: Vec<Bundle<WithOtherFields<TransactionRequest>>>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
        state_override: Option<alloy_rpc_types::state::StateOverride>,
    ) -> Result<Vec<Vec<EthCallResponse>>, BlockchainError> {
        if bundles.is_empty() {
            return Err(BlockchainError::RpcError(RpcError::invalid_params(
                "bundles are empty.".to_string(),
            )));
        }

        self.with_database_at_and_context(
            block_request,
            |state, mut block_env, mut monad_context| {
                let mut cache_db = CacheDB::new(&*state);
                if let Some(state_override) = state_override {
                    apply_state_overrides(state_override, &mut cache_db)?;
                }

                let mut results = Vec::with_capacity(bundles.len());
                for bundle in bundles {
                    let Bundle { transactions, block_override } = bundle;
                    if let Some(block_override) = block_override {
                        apply_block_overrides(block_override, &mut cache_db, &mut block_env);
                    }

                    let mut bundle_results = Vec::with_capacity(transactions.len());
                    for request in transactions {
                        let fee_details = FeeDetails::new(
                            request.gas_price,
                            request.max_fee_per_gas,
                            request.max_priority_fee_per_gas,
                            request.max_fee_per_blob_gas,
                        )?
                        .or_zero_fees();
                        let PreparedCall { evm_env, mut tx, simulated_tempo_tx, .. } = self
                            .prepare_call_env(&cache_db, request, fee_details, block_env.clone())?;
                        apply_tempo_envelope_identity(&mut tx, simulated_tempo_tx.as_ref());

                        let mut inspector = self.build_inspector();
                        let Executed { result, state } = self.transact_call_with_inspector_ref(
                            &cache_db,
                            &evm_env,
                            &mut inspector,
                            &tx,
                            monad_context.as_mut().map(next_monad_context),
                        )?;

                        let output = result.output.clone();
                        let response = if result.status {
                            EthCallResponse { value: Some(output), error: None }
                        } else {
                            let error = RevertDecoder::new()
                                .maybe_decode(&output, None)
                                .unwrap_or_else(|| "execution failed".to_string());
                            EthCallResponse { value: None, error: Some(error) }
                        };

                        commit_pending(&mut cache_db, &state);
                        bundle_results.push(response);
                    }

                    results.push(bundle_results);
                    block_env.number = block_env.number.saturating_add(U256::ONE);
                    block_env.timestamp = block_env.timestamp.saturating_add(U256::ONE);
                    #[cfg(feature = "monad")]
                    self::monad::advance_block_context(&mut monad_context);
                }

                Ok(results)
            },
        )
        .await
    }

    /// Simulates the payload by executing the calls in request.
    pub async fn simulate(
        &self,
        request: SimulatePayload,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
        block_interval: u64,
    ) -> Result<Vec<SimulatedBlock<AnyRpcBlock>>, BlockchainError> {
        self.simulate_raw(
            preserve_simulation_request_fields(request),
            block_request,
            block_interval,
        )
        .await
    }

    /// Simulates a payload while preserving transaction extension fields.
    pub(crate) async fn simulate_raw(
        &self,
        request: SimulatePayload<WithOtherFields<TransactionRequest>>,
        block_request: Option<BlockRequest<FoundryTxEnvelope>>,
        block_interval: u64,
    ) -> Result<Vec<SimulatedBlock<AnyRpcBlock>>, BlockchainError> {
        let simulate_at = |state: Box<dyn MaybeFullDatabase + '_>,
                           base_block_env: BlockEnv,
                           base_number,
                           base_timestamp,
                           base_hash,
                           base_fee,
                           mut monad_context: Option<MonadReplayContext>,
                           base_base_fee_per_gas,
                           base_excess_blob_gas,
                           base_blob_gas_used,
                           base_fee_extra_data: Bytes,
                           optimism_jovian: bool| {
            let SimulatePayload {
                block_state_calls,
                trace_transfers,
                validation,
                return_full_transactions,
            } = request;
            let block_state_calls = sanitize_simulation_blocks(
                block_state_calls,
                base_number,
                base_timestamp,
                block_interval,
            )?;
            let mut cache_db = CacheDB::new(state);
            cache_db.cache.block_hashes.insert(U256::from(base_number), base_hash);
            let mut block_res = Vec::with_capacity(block_state_calls.len());
            // A single block cannot amortize building an incremental trie.
            let cache_state_roots = block_state_calls.len() > 1;
            let mut state_root_cache = None::<(StateRootCache, AddressMap<DbAccount>)>;
            let mut parent_hash = base_hash;
            let mut next_base_fee = base_fee;
            let mut inherited_block_env = base_block_env;
            let (is_merge, is_cancun, is_amsterdam, tx_gas_limit_cap) = {
                let cfg_env = &self.evm_env.read().cfg_env;
                (
                    cfg_env.spec >= SpecId::MERGE,
                    cfg_env.spec >= SpecId::CANCUN,
                    cfg_env.spec >= SpecId::AMSTERDAM,
                    cfg_env.tx_gas_limit_cap(),
                )
            };
            let mut parent_base_fee_per_gas = base_base_fee_per_gas;
            let mut parent_excess_blob_gas = base_excess_blob_gas;
            let mut parent_blob_gas_used = base_blob_gas_used;
            let mut rpc_gas_budget = SIMULATE_GAS_CAP;

            // execute the blocks
            for block in block_state_calls {
                let SimBlock { block_overrides, state_overrides, calls } = block;
                let mut block_env = inherited_block_env.clone();
                let overridden_beacon_root =
                    block_overrides.as_ref().and_then(|overrides| overrides.beacon_root);
                let block_timestamp = block_overrides
                    .as_ref()
                    .and_then(|overrides| overrides.time)
                    .unwrap_or_else(|| block_env.timestamp.saturating_to());
                let blob_params = self.simulation_blob_params_at_timestamp(block_timestamp);
                if is_cancun {
                    let excess_blob_gas = self.networks.next_block_blob_excess_gas(
                        blob_params,
                        parent_excess_blob_gas,
                        parent_blob_gas_used,
                        parent_base_fee_per_gas,
                    );
                    block_env.set_blob_excess_gas_and_price(
                        excess_blob_gas,
                        blob_params.update_fraction as u64,
                    );
                } else {
                    block_env.blob_excess_gas_and_price = None;
                }
                block_env.basefee = if validation { next_base_fee } else { 0 };
                block_env.prevrandao = Some(B256::ZERO);
                if is_merge && !is_arbitrum(self.protocol_chain_id()) {
                    block_env.difficulty = U256::ZERO;
                }
                let mut call_res = Vec::with_capacity(calls.len());
                let mut log_index = 0;
                let mut cumulative_gas_used = 0;
                let mut block_regular_gas_used = 0;
                let mut block_state_gas_used = 0;
                let mut block_blob_gas_used = 0u64;
                let mut transactions = Vec::with_capacity(calls.len());
                let mut transaction_envelopes = Vec::with_capacity(calls.len());
                let mut receipts = Vec::with_capacity(calls.len());
                let overridden_block_hashes = block_overrides
                    .as_ref()
                    .and_then(|overrides| overrides.block_hash.as_ref())
                    .map(|overrides| {
                        overrides
                            .keys()
                            .map(|number| {
                                let number = U256::from(*number);
                                (number, cache_db.cache.block_hashes.get(&number).copied())
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                if let Some(block_overrides) = block_overrides {
                    apply_block_overrides(block_overrides, &mut cache_db, &mut block_env);
                }
                // Simulation overrides and RPC results use L2 numbers, while Arbitrum
                // execution reads the corresponding L1 number through NUMBER.
                let rpc_block_number = block_env.number.saturating_to();
                block_env.number = self.evm_block_number(rpc_block_number);
                let simulation_evm_env =
                    EvmEnv::new(self.evm_env.read().cfg_env.clone(), block_env.clone());
                let spec_id = *simulation_evm_env.spec_id();
                let ethereum_transitions = self
                    .ethereum_block_transitions(self.hardfork(), None, BlockExecutionKind::Complete)
                    .map(|mut transitions| {
                        transitions.parent_beacon_block_root = (transitions.hardfork
                            >= EthereumHardfork::Cancun)
                            .then_some(overridden_beacon_root.unwrap_or_default());
                        transitions
                    });
                let precompile_overrides = self.simulation_precompile_overrides(
                    state_overrides.as_ref(),
                    &simulation_evm_env,
                )?;

                // Apply state overrides after validating precompile moves against this block's
                // active precompile set.
                if let Some(mut state_overrides) = state_overrides {
                    state_overrides.retain(|_, account| {
                        account.balance.is_some()
                            || account.nonce.is_some()
                            || account.code.is_some()
                            || account.state.is_some()
                            || account
                                .state_diff
                                .as_ref()
                                .is_some_and(|state_diff| !state_diff.is_empty())
                    });
                    let previously_deleted = previously_deleted_accounts(
                        &cache_db.cache.accounts,
                        state_overrides.keys().copied(),
                    );
                    apply_state_overrides(state_overrides, &mut cache_db)?;
                    preserve_deleted_storage(&mut cache_db.cache.accounts, previously_deleted);
                }

                // Overrides define the starting state, so only execution contributes BAL writes.
                let mut block_access_list = if let Some(transitions) = ethereum_transitions {
                    self.apply_simulation_pre_execution_changes(
                        &mut cache_db,
                        &simulation_evm_env,
                        parent_hash,
                        transitions,
                    )?
                } else {
                    BalContext::default()
                };

                // execute all calls in that block
                for (req_idx, mut request) in calls.into_iter().enumerate() {
                    #[cfg(feature = "base")]
                    if request.transaction_type == Some(base_common_consensus::EIP8130_TX_TYPE_ID) {
                        return Err(BlockchainError::InvalidTransactionRequest(
                            "eth_simulateV1 does not accept EIP-8130 transaction requests"
                                .to_string(),
                        ));
                    }

                    let classified_request = self.parse_transaction_request(request.clone())?;
                    #[cfg(feature = "base")]
                    if classified_request.is_base() {
                        return Err(BlockchainError::InvalidTransactionRequest(
                            "eth_simulateV1 does not accept EIP-8130 transaction requests"
                                .to_string(),
                        ));
                    }
                    let is_ethereum_request = classified_request.is_ethereum();
                    let mut parsed_request = self.is_tempo().then_some(classified_request);
                    if is_ethereum_request {
                        request.populate_blob_hashes();
                        let preferred_type = request.preferred_type();
                        request.transaction_type = Some(preferred_type as u8);
                        request.trim_conflicting_keys();
                        request.populate_blob_hashes();
                    }
                    let request_blob_gas_used = if is_ethereum_request && !optimism_jovian {
                        u64::try_from(request.blob_versioned_hashes.as_ref().map_or(0, Vec::len))
                            .unwrap_or(u64::MAX)
                            .saturating_mul(DATA_GAS_PER_BLOB)
                    } else {
                        0
                    };
                    let max_blob_gas = crate::eth::backend::executor::block_blob_gas_limit(
                        optimism_jovian,
                        block_env.gas_limit,
                        blob_params.max_blob_gas_per_block(),
                    );
                    if !optimism_jovian
                        && block_blob_gas_used.saturating_add(request_blob_gas_used) > max_blob_gas
                    {
                        return Err(BlockchainError::RpcError(RpcError::invalid_params(format!(
                            "blob gas usage exceeds the limit of {max_blob_gas} gas per block."
                        ))));
                    }
                    block_blob_gas_used = block_blob_gas_used.saturating_add(request_blob_gas_used);

                    let inner = request.as_ref();
                    let remaining_regular_gas =
                        block_env.gas_limit.saturating_sub(block_regular_gas_used);
                    let remaining_state_gas =
                        block_env.gas_limit.saturating_sub(block_state_gas_used);
                    let remaining_gas = if is_amsterdam {
                        remaining_regular_gas.min(remaining_state_gas)
                    } else {
                        block_env.gas_limit.saturating_sub(cumulative_gas_used)
                    };
                    let requested_gas = inner.gas.unwrap_or(remaining_gas);
                    let exceeds_gas_limit = if is_amsterdam {
                        let requested_regular_gas = requested_gas.min(tx_gas_limit_cap);
                        requested_regular_gas > remaining_regular_gas
                            || requested_gas > remaining_state_gas
                    } else {
                        requested_gas > remaining_gas
                    };
                    if exceeds_gas_limit {
                        return Err(BlockchainError::RpcError(RpcError {
                            code: ErrorCode::ServerError(-38015),
                            message: format!(
                                "block gas limit exceeded: remaining {remaining_gas}, requested {requested_gas}"
                            )
                            .into(),
                            data: None,
                        }));
                    }
                    let execution_gas_limit = requested_gas.min(rpc_gas_budget);
                    let preserve_signed_gas = matches!(
                        &parsed_request,
                        Some(FoundryTransactionRequest::Tempo(request))
                            if request.fee_payer_signature.is_some()
                    );
                    request.gas = Some(execution_gas_limit);
                    if !preserve_signed_gas && let Some(parsed_request) = &mut parsed_request {
                        parsed_request.as_mut().gas = Some(execution_gas_limit);
                    }

                    let caller = request.from.unwrap_or_default();
                    let caller_nonce = cache_db
                        .basic_ref(caller)?
                        .map(|account| account.nonce)
                        .unwrap_or_default();
                    let tempo_nonce_key =
                        parsed_request.as_ref().and_then(|request| match request {
                            FoundryTransactionRequest::Tempo(request) => request.nonce_key,
                            _ => None,
                        });
                    if request.nonce.is_none() {
                        let nonce = tempo_nonce_key.map_or(Ok(caller_nonce), |nonce_key| {
                            tempo_nonce(&cache_db, caller, nonce_key)
                        })?;
                        request.nonce = Some(nonce);
                        if let Some(parsed_request) = &mut parsed_request {
                            parsed_request.as_mut().nonce = Some(nonce);
                        }
                    }

                    if is_ethereum_request {
                        let mut canonical_request =
                            FoundryTransactionRequest::Ethereum(request.inner.clone());
                        canonical_request.prep_for_submission();
                        request.inner = canonical_request.as_ref().clone();
                        if let Some(parsed_request) = &mut parsed_request {
                            *parsed_request = canonical_request;
                        }
                    }

                    let fee_details = FeeDetails::new(
                        request.gas_price,
                        request.max_fee_per_gas,
                        request.max_priority_fee_per_gas,
                        request.max_fee_per_blob_gas,
                    )?
                    .or_zero_fees();

                    let PreparedCall {
                        mut evm_env,
                        mut tx,
                        simulated_tempo_tx,
                        #[cfg(feature = "base")]
                        simulated_envelope,
                    } = if let Some(parsed_request) = parsed_request {
                        self.prepare_typed_call_env(
                            &cache_db,
                            parsed_request,
                            fee_details,
                            block_env.clone(),
                        )?
                    } else {
                        self.prepare_call_env(
                            &cache_db,
                            request.clone(),
                            fee_details,
                            block_env.clone(),
                        )?
                    };
                    tx.tx = with_gas_limit(tx.tx, execution_gas_limit);
                    apply_tempo_envelope_identity(&mut tx, simulated_tempo_tx.as_ref());
                    if !validation
                        && tempo_nonce_key.is_none_or(|key| key.is_zero())
                        && request.nonce == Some(u64::MAX)
                    {
                        tx.tx = with_nonce(tx.tx, 0);
                    }
                    let uses_protocol_call_nonce = uses_protocol_call_nonce(&tx.tx);
                    #[cfg(feature = "base")]
                    let simulated_envelope = simulated_envelope
                        .or_else(|| simulated_tempo_tx.map(FoundryTxEnvelope::Tempo));
                    #[cfg(not(feature = "base"))]
                    let simulated_envelope = simulated_tempo_tx.map(FoundryTxEnvelope::Tempo);

                    // Always disable EIP-3607
                    evm_env.cfg_env.disable_eip3607 = true;

                    // Simulated blocks keep their own base and blob base fees, which are what
                    // BASEFEE and BLOBBASEFEE read and what validation checks fees against.
                    evm_env.block_env.basefee = block_env.basefee;
                    evm_env.block_env.blob_excess_gas_and_price =
                        block_env.blob_excess_gas_and_price;
                    // Without validation, calls run at any price against the simulated block's
                    // fees.
                    evm_env.cfg_env.disable_base_fee = !validation;
                    // Only zero-cap blob calls bypass blob pricing; an explicit cap still pays
                    // the simulated block's blob fee and must cover it.
                    evm_env.cfg_env.disable_blob_base_fee =
                        !validation && request.max_fee_per_blob_gas.unwrap_or_default() == 0;
                    if validation {
                        evm_env.cfg_env.disable_nonce_check = false;
                        evm_env.cfg_env.disable_block_gas_limit = false;
                    }

                    let mut inspector = self.build_inspector();

                    // transact
                    inspector = inspector.with_simulation_logs(trace_transfers);
                    trace!(target: "backend", env=?evm_env, spec=?evm_env.spec_id(),"simulate evm env");
                    let mut opts = self.evm_options(&evm_env);
                    opts.precompile_moves = precompile_overrides.moves.clone();
                    let context =
                        self.tx_context(monad_context.as_mut().map(next_monad_context), &tx, false);
                    let execution_result =
                        self.execute_tx(&cache_db, &evm_env, &opts, &tx, &context, &mut inspector);
                    let Executed { result, mut state } = match execution_result {
                        Err(BlockchainError::InvalidTransaction(error)) => {
                            return Err(simulate_transaction_error(error));
                        }
                        result => result?,
                    };
                    if !validation
                        && caller_nonce == u64::MAX
                        && uses_protocol_call_nonce
                        && let Some(info) = state.account_info(&caller).cloned()
                    {
                        let original = cache_db.basic_ref(caller)?;
                        state.insert_account(
                            caller,
                            original,
                            Some(AccountInfo { nonce: 0, ..info }),
                        );
                    }
                    trace!(target: "backend", ?result, ?request, "simulate call");

                    let canonical_logs = result.logs.clone();
                    let (response_logs, attempted_log_count) = inspector
                        .take_simulation_logs(&canonical_logs, result.status)
                        .expect("simulation log collector is installed");
                    inspector.print_logs();
                    if self.print_traces {
                        inspector.into_print_traces(self.call_trace_decoder());
                    }

                    // REVM turns a previously deleted account into `Touched` when a later call
                    // recreates it without storage. Preserve the cleared-storage provenance so
                    // subsequent calls and the recursively merged state cannot reload old slots.
                    let previously_deleted = previously_deleted_accounts(
                        &cache_db.cache.accounts,
                        state.account_changes().map(|changes| changes.address),
                    );

                    rpc_gas_budget = rpc_gas_budget.saturating_sub(result.tx_gas_used());
                    cumulative_gas_used = cumulative_gas_used.saturating_add(result.tx_gas_used());
                    block_regular_gas_used =
                        block_regular_gas_used.saturating_add(result.execution_gas_spent());
                    block_state_gas_used =
                        block_state_gas_used.saturating_add(result.state_gas_spent());

                    // create the transaction from a request
                    let from = caller;
                    request.sidecar = None;
                    let tx = if let Some(envelope) = simulated_envelope {
                        MaybeImpersonatedTransaction::impersonated(envelope, from)
                    } else {
                        if request.to.is_none() {
                            request.to = Some(TxKind::Create);
                        }
                        let mut request = self.parse_transaction_request(request)?;
                        request.prep_for_submission();
                        let typed_tx = request.build_unsigned().map_err(|e| {
                            BlockchainError::InvalidTransactionRequest(e.to_string())
                        })?;
                        MaybeImpersonatedTransaction::impersonated(
                            typed_tx.into_impersonated(),
                            from,
                        )
                    };
                    let tx_hash = tx.as_ref().hash();
                    #[cfg(feature = "optimism")]
                    if optimism_jovian {
                        let tx_blob_gas = {
                            let mut evm = build_evm::<crate::eth::backend::evm::OpEvmNetwork>(
                                &cache_db, &evm_env, &opts,
                            )?;
                            crate::eth::backend::executor::optimism::blob_gas_used(
                                &mut evm,
                                tx.as_ref(),
                                true,
                            )?
                        };
                        if block_blob_gas_used.saturating_add(tx_blob_gas) > max_blob_gas {
                            return Err(BlockchainError::RpcError(RpcError::invalid_params(
                                format!(
                                    "blob gas usage exceeds the limit of {max_blob_gas} gas per block."
                                ),
                            )));
                        }
                        block_blob_gas_used = block_blob_gas_used.saturating_add(tx_blob_gas);
                    }

                    // Commit after calculating the footprint so the scalar comes from pre-tx
                    // state, matching the upstream OP block executor.
                    commit_pending(&mut cache_db, &state);
                    block_access_list.set_bal_index(BlockAccessIndex::new(req_idx as u64 + 1));
                    block_access_list.commit_pending(&state);
                    preserve_deleted_storage(&mut cache_db.cache.accounts, previously_deleted);
                    #[cfg(any(feature = "base", feature = "optimism"))]
                    let receipt = if tx.as_ref().is_deposit() {
                        crate::eth::backend::executor::optimism::build_deposit_receipt(
                            result.status,
                            canonical_logs.clone(),
                            Some(self.hardfork()),
                            caller_nonce,
                            cumulative_gas_used,
                        )
                    } else {
                        build_receipt(
                            tx.as_ref().tx_type(),
                            result.status,
                            canonical_logs.clone(),
                            cumulative_gas_used,
                            result.ext.eip8130_phase_statuses.clone(),
                        )
                    };
                    #[cfg(not(any(feature = "base", feature = "optimism")))]
                    let receipt = build_receipt(
                        tx.as_ref().tx_type(),
                        result.status,
                        canonical_logs.clone(),
                        cumulative_gas_used,
                        result.ext.eip8130_phase_statuses.clone(),
                    );
                    receipts.push(receipt);
                    transaction_envelopes.push(tx.as_ref().clone());
                    let rpc_tx =
                        transaction_build(Some(tx_hash), tx, None, None, Some(block_env.basefee));
                    transactions.push(rpc_tx);

                    let return_data =
                        if result.status { result.output.clone() } else { Bytes::new() };
                    let sim_res = SimCallResult {
                        return_data,
                        gas_used: result.tx_gas_used(),
                        max_used_gas: Some(result.total_gas_spent.max(result.floor_gas)),
                        status: result.status,
                        error: if result.status {
                            None
                        } else if result.stop.is_revert() && !result.ext.failed_deposit {
                            let output = &result.output;
                            let message = RevertDecoder::new()
                                .maybe_decode(output, None)
                                .map(|reason| format!("execution reverted: {reason}"))
                                .unwrap_or_else(|| "execution reverted".to_string());
                            Some(SimulateError {
                                code: SimulateError::EXECUTION_REVERTED_CODE,
                                message,
                                data: Some(output.clone()),
                            })
                        } else {
                            Some(SimulateError {
                                code: SimulateError::VM_EXECUTION_ERROR_CODE,
                                message: if result.stop.is_out_of_gas() {
                                    "out of gas".to_string()
                                } else {
                                    format!("vm execution error: {}", halt_reason(result.stop))
                                },
                                data: None,
                            })
                        },
                        logs: response_logs
                            .into_iter()
                            .map(|(idx, log)| Log {
                                inner: log,
                                block_number: Some(rpc_block_number),
                                block_timestamp: Some(block_env.timestamp.saturating_to()),
                                transaction_index: Some(req_idx as u64),
                                log_index: Some(idx + log_index),
                                removed: false,

                                block_hash: None,
                                transaction_hash: Some(tx_hash),
                            })
                            .collect(),
                    };
                    log_index += attempted_log_count;
                    call_res.push(sim_res);
                }

                for (number, hash) in overridden_block_hashes {
                    if let Some(hash) = hash {
                        cache_db.cache.block_hashes.insert(number, hash);
                    } else {
                        cache_db.cache.block_hashes.remove(&number);
                    }
                }

                let gas_used = if is_amsterdam {
                    block_regular_gas_used.max(block_state_gas_used)
                } else {
                    cumulative_gas_used
                };
                let (requests, block_access_list) = if let Some(transitions) = ethereum_transitions
                {
                    self.apply_simulation_post_execution_changes(
                        &mut cache_db,
                        &simulation_evm_env,
                        transitions,
                        &receipts,
                        block_access_list,
                    )?
                } else {
                    Default::default()
                };

                // Fork databases are partial, so their synthetic blocks use a zero state root.
                let incremental = state_root_cache.as_mut().is_some_and(|(cache, previous)| {
                    cache.record_overlay(&cache_db.cache.accounts, previous)
                });
                let state_root = if incremental {
                    let (cache, previous) = state_root_cache.as_mut().unwrap();
                    let root = cache.root(&cache_db.cache.accounts);
                    previous.clone_from(&cache_db.cache.accounts);
                    root
                } else {
                    cache_db
                        .maybe_full_db()
                        .map(|accounts| {
                            // A deleted base account can retain storage written by
                            // anvil_setStorageAt. A later touch exposes it in the full merge,
                            // although it has no trie leaf. Keep the full rebuild in that case.
                            if cache_state_roots
                                && accounts.values().all(|account| {
                                    account.account_state != AccountState::NotExisting
                                        || account.storage.is_empty()
                                })
                            {
                                let (cache, previous) =
                                    state_root_cache.get_or_insert_with(Default::default);
                                let root = cache.root(&accounts);
                                previous.clone_from(&cache_db.cache.accounts);
                                root
                            } else {
                                state_root_cache = None;
                                state_root(&accounts)
                            }
                        })
                        .unwrap_or_default()
                };
                let block_access_list_hash =
                    block_access_list.map(|bal| compute_block_access_list_hash(bal.as_slice()));
                let header = Header {
                    block_access_list_hash,
                    logs_bloom: receipts.iter().fold(Bloom::ZERO, |mut bloom, receipt| {
                        bloom.accrue_bloom(receipt.logs_bloom());
                        bloom
                    }),
                    transactions_root: calculate_transaction_root(&transaction_envelopes),
                    receipts_root: calculate_receipt_root(&receipts),
                    parent_hash,
                    beneficiary: block_env.beneficiary,
                    state_root,
                    difficulty: block_env.difficulty,
                    number: rpc_block_number,
                    gas_limit: block_env.gas_limit,
                    gas_used,
                    timestamp: block_env.timestamp.saturating_to(),
                    extra_data: base_fee_extra_data.clone(),
                    mix_hash: block_env.prevrandao.unwrap_or_default(),
                    nonce: Default::default(),
                    base_fee_per_gas: (spec_id >= SpecId::LONDON).then_some(block_env.basefee),
                    withdrawals_root: (spec_id >= SpecId::SHANGHAI).then_some(EMPTY_WITHDRAWALS),
                    blob_gas_used: is_cancun.then_some(block_blob_gas_used),
                    excess_blob_gas: if is_cancun { block_env.blob_excess_gas() } else { None },
                    parent_beacon_block_root: ethereum_transitions.and_then(|transitions| {
                        (transitions.hardfork >= EthereumHardfork::Cancun)
                            .then_some(transitions.parent_beacon_block_root.unwrap_or_default())
                    }),
                    requests_hash: ethereum_transitions.and_then(|transitions| {
                        (transitions.hardfork >= EthereumHardfork::Prague)
                            .then(|| requests.requests_hash())
                    }),
                    ..Default::default()
                };
                let block_hash = header.hash_slow();
                let withdrawals = (spec_id >= SpecId::SHANGHAI).then_some(Default::default());
                let size = U256::from(
                    BlockBody {
                        transactions: transaction_envelopes,
                        ommers: vec![],
                        withdrawals: withdrawals.clone(),
                    }
                    .into_block(header.clone())
                    .length(),
                );
                for (transaction_index, transaction) in transactions.iter_mut().enumerate() {
                    transaction.block_hash = Some(block_hash);
                    transaction.block_number = Some(header.number);
                    transaction.transaction_index = Some(transaction_index as u64);
                    transaction.block_timestamp = Some(header.timestamp);
                }
                let mut block = alloy_rpc_types::Block {
                    header: AnyRpcHeader {
                        hash: block_hash,
                        inner: header.into(),
                        total_difficulty: None,
                        size: Some(size),
                    },
                    uncles: vec![],
                    transactions: BlockTransactions::Full(transactions),
                    withdrawals,
                };

                if !return_full_transactions {
                    block.transactions.convert_to_hashes();
                }

                for res in &mut call_res {
                    res.logs.iter_mut().for_each(|log| {
                        log.block_hash = Some(block.header.hash);
                    });
                }

                let mut block = AnyRpcBlock::new(WithOtherFields::new(block));
                if is_arbitrum(self.protocol_chain_id()) {
                    block
                        .other
                        .insert("l1BlockNumber".to_string(), serde_json::json!(block_env.number));
                }
                let simulated_block = SimulatedBlock { inner: block, calls: call_res };

                parent_hash = block_hash;
                cache_db.cache.block_hashes.insert(U256::from(rpc_block_number), block_hash);
                inherited_block_env.beneficiary = block_env.beneficiary;
                inherited_block_env.difficulty = block_env.difficulty;
                inherited_block_env.gas_limit = block_env.gas_limit;
                // Route through the fee manager so Tempo chains use their own base fee rules.
                let header = &simulated_block.inner.header;
                next_base_fee = self.fees.calculate_next_block_base_fee_from_header(&header.inner);
                parent_base_fee_per_gas = header.base_fee_per_gas().unwrap_or_default();
                parent_excess_blob_gas = header.excess_blob_gas().unwrap_or_default();
                parent_blob_gas_used = header.blob_gas_used().unwrap_or_default();

                block_res.push(simulated_block);
                #[cfg(feature = "monad")]
                self::monad::advance_block_context(&mut monad_context);
            }

            Ok(block_res)
        };

        match block_request {
            Some(BlockRequest::Pending(pool_transactions)) => {
                self.with_pending_block(pool_transactions, |state, block| {
                    let header = &block.block.header;
                    let parent_fees = self.fees.calculate_parent_header_fees(header);
                    let optimism_jovian =
                        self.is_optimism_jovian_at_header(header, parent_fees.optimism_jovian);
                    let monad_context = self.active_monad_context_before_mined_transaction(
                        &block.block,
                        block.block.body.transactions.len(),
                    )?;
                    #[cfg(feature = "monad")]
                    let monad_context = {
                        let mut monad_context = monad_context;
                        self::monad::advance_block_context(&mut monad_context);
                        monad_context
                    };
                    simulate_at(
                        state,
                        self.block_env_from_header(header),
                        header.number(),
                        header.timestamp(),
                        header.hash_slow(),
                        parent_fees.base_fee,
                        monad_context,
                        header.base_fee_per_gas().unwrap_or_default(),
                        header.excess_blob_gas().unwrap_or_default(),
                        header.blob_gas_used().unwrap_or_default(),
                        parent_fees.extra_data,
                        optimism_jovian,
                    )
                })
                .await?
            }
            block_request => {
                let base_block_number = match block_request.as_ref() {
                    Some(BlockRequest::Number(number)) => BlockNumber::Number(*number),
                    Some(BlockRequest::Pending(_)) => unreachable!(),
                    None => BlockNumber::Latest,
                };
                let base_block = self
                    .block_by_number(base_block_number)
                    .await?
                    .ok_or(BlockchainError::BlockNotFound)?;
                let base_number = base_block.header.number();
                let base_timestamp = base_block.header.timestamp();
                let base_hash = base_block.header.hash;
                let parent_fees = self.fees.calculate_parent_header_fees(&base_block.header.inner);
                let optimism_jovian = self.is_optimism_jovian_at_header(
                    &base_block.header.inner,
                    parent_fees.optimism_jovian,
                );

                #[cfg(feature = "monad")]
                let monad_context = if self.is_monad() {
                    Some(self.monad_context_for_child_of_block_number(base_number).await?)
                } else {
                    None
                };
                #[cfg(not(feature = "monad"))]
                let monad_context = None;

                self.with_database_at(block_request, |state, block_env| {
                    simulate_at(
                        state,
                        block_env,
                        base_number,
                        base_timestamp,
                        base_hash,
                        parent_fees.base_fee,
                        monad_context,
                        base_block.header.base_fee_per_gas().unwrap_or_default(),
                        base_block.header.excess_blob_gas().unwrap_or_default(),
                        base_block.header.blob_gas_used().unwrap_or_default(),
                        parent_fees.extra_data,
                        optimism_jovian,
                    )
                })
                .await?
            }
        }
    }

    pub fn get_blob_by_tx_hash(&self, hash: B256) -> Result<Option<Vec<alloy_consensus::Blob>>> {
        let storage = self.blockchain.storage.read();
        Ok(storage.transactions.get(&hash).and_then(|mined| {
            storage
                .blocks
                .get(&mined.block_hash)?
                .body
                .transactions
                .get(mined.info.transaction_index as usize)?
                .as_ref()
                .sidecar()
                .map(|sidecar| sidecar.sidecar.blobs().to_vec())
        }))
    }

    /// Sets the fee token for a user address (Tempo-only).
    pub async fn set_fee_token(&self, user: Address, token: Address) -> DatabaseResult<()> {
        self.with_tempo_storage(|| {
            let mut fee_manager = TipFeeManager::new();
            fee_manager
                .set_user_token(user, IFeeManager::setUserTokenCall { token })
                .map_err(tempo_db_err)
        })
        .await
    }

    /// Sets the fee token for a validator address (Tempo-only).
    pub async fn set_validator_fee_token(
        &self,
        validator: Address,
        token: Address,
    ) -> DatabaseResult<()> {
        self.with_tempo_storage(|| {
            let mut fee_manager = TipFeeManager::new();
            // Use Address::ZERO as beneficiary so the check `sender != beneficiary` passes
            fee_manager
                .set_validator_token(
                    validator,
                    IFeeManager::setValidatorTokenCall { token },
                    Address::ZERO,
                )
                .map_err(tempo_db_err)
        })
        .await
    }

    /// Mints FeeAMM liquidity for a token pair (Tempo-only).
    pub async fn set_fee_amm_liquidity(
        &self,
        user_token: Address,
        validator_token: Address,
        amount: U256,
    ) -> DatabaseResult<()> {
        // T3+ rejects minting to the zero address.
        let admin = Address::repeat_byte(0x11);
        self.with_tempo_storage(|| {
            // Mint the required tokens to admin so it can provide liquidity.
            // grant_role_internal bypasses the caller check, matching genesis seeding.
            for &token_address in &[user_token, validator_token] {
                let mut token = TIP20Token::from_address(token_address).map_err(tempo_db_err)?;
                token
                    .grant_role_internal(admin, TIP20Token::issuer_role())
                    .map_err(tempo_db_err)?;
                token.mint(admin, ITIP20::mintCall { to: admin, amount }).map_err(tempo_db_err)?;
            }
            let mut fee_manager = TipFeeManager::new();
            fee_manager
                .mint(admin, user_token, validator_token, amount, admin)
                .map_err(tempo_db_err)?;
            Ok(())
        })
        .await
    }

    /// Sets an account's balance for a deployed TIP-20 token (Tempo-only).
    pub async fn set_tip20_balance(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> DatabaseResult<()> {
        if self.try_set_tip20_balance(address, token_address, balance).await? {
            return Ok(());
        }

        Err(tempo_db_err(format!("address {token_address} is not a deployed TIP-20 token")))
    }

    /// Sets an account's balance if the address is a deployed TIP-20 token (Tempo-only).
    pub async fn try_set_tip20_balance(
        &self,
        address: Address,
        token_address: Address,
        balance: U256,
    ) -> DatabaseResult<bool> {
        self.with_tempo_storage(|| {
            if !TIP20Factory::new().is_tip20(token_address).map_err(tempo_db_err)? {
                return Ok(false);
            }

            let mut token = TIP20Token::from_address(token_address).map_err(tempo_db_err)?;
            token.balances[address].write(balance).map_err(tempo_db_err)?;
            Ok(true)
        })
        .await
    }

    /// Runs `f` inside a Tempo storage context initialized from the current state and writes
    /// its changes back (Tempo-only).
    async fn with_tempo_storage<R>(
        &self,
        f: impl FnOnce() -> Result<R, DatabaseError>,
    ) -> Result<R, DatabaseError> {
        let hardfork = self.hardfork();
        let env = self.evm_env.read().clone();
        let mut db = self.db.write().await;
        crate::eth::backend::tempo::with_tempo_storage(&mut **db, &env, hardfork, f)
            .map_err(tempo_db_err)?
    }
}

/// Returns the genesis environment Tempo precompiles are initialized in.
fn tempo_genesis_env(chain_id: u64, timestamp: u64, hardfork: TempoHardfork) -> EvmEnv {
    EvmEnv {
        cfg_env: CfgEnv {
            chain_id,
            spec: FoundryHardfork::from(hardfork).into(),
            ..Default::default()
        },
        block_env: BlockEnv { timestamp: U256::from(timestamp), ..Default::default() },
    }
}

/// Converts a Tempo error into an anvil [`DatabaseError`].
fn tempo_db_err<E: std::fmt::Display>(e: E) -> DatabaseError {
    DatabaseError::AnyRequest(Arc::new(eyre::eyre!("{e}")))
}

/// Get max nonce from transaction pool by address.
fn get_pool_transactions_nonce(
    pool_transactions: &[Arc<PoolTransaction<FoundryTxEnvelope>>],
    address: Address,
) -> Option<u64> {
    if let Some(highest_nonce) = pool_transactions
        .iter()
        .filter(|tx| {
            *tx.pending_transaction.sender() == address
                && !tx.pending_transaction.transaction.as_ref().has_nonzero_tempo_nonce_key()
        })
        .map(|tx| tx.pending_transaction.nonce())
        .max()
    {
        let tx_count = highest_nonce.saturating_add(1);
        return Some(tx_count);
    }
    None
}

impl<N: Network> Backend<N>
where
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    /// Validates a transaction candidate selected for mining.
    fn validate_mining_pool_transaction_for(
        &self,
        pool_tx: &PoolTransaction<FoundryTxEnvelope>,
        account: &AccountInfo,
        evm_env: &EvmEnv,
    ) -> Result<(), InvalidTransactionError> {
        #[cfg(feature = "monad")]
        if self.validate_monad_mining_pool_transaction_for(pool_tx) {
            return Ok(());
        }

        self.validate_pool_transaction_for(&pool_tx.pending_transaction, account, evm_env)
    }

    /// Runs Tempo's transaction-pool validation for `pending` against the latest state.
    ///
    /// This is the pipeline the node runs before admitting a transaction: it resolves the fee
    /// token the way execution will, including a token inferred from the called TIP20 or DEX
    /// swap and a preference the transaction itself sets, charges intrinsic gas, and checks the
    /// fee payer's balance against the maximum fee with execution's rounding, all without
    /// committing state.
    async fn validate_tempo_pool_transaction(
        &self,
        pending: &PendingTransaction<FoundryTxEnvelope>,
        evm_env: &EvmEnv,
    ) -> Result<(), BlockchainError> {
        let tx = PreparedTx::signed(pending.transaction.as_ref().clone(), *pending.sender());
        let evm_env = evm_env.clone();
        let opts = self.evm_options(&evm_env);
        self.with_database_at(None, move |state, _| {
            let mut evm = build_evm::<TempoEvmNetwork>(&*state, &evm_env, &opts)?;
            // Pool admission does not require the lower validity bound or fee AMM liquidity.
            evm.ext_mut().skip_valid_after_check = true;
            evm.ext_mut().skip_liquidity_check = true;
            match crate::eth::backend::evm::transact::<TempoEvmNetwork>(
                &mut evm,
                &tx,
                &TxContext::default(),
                opts.cheats.as_ref(),
            ) {
                Ok(_) => Ok(()),
                // The pool queues transactions ahead of their nonce lane, and fee AMM liquidity
                // is only required at inclusion; evm2 has no validation-only entry point.
                Err(BlockchainError::InvalidTransaction(InvalidTransactionError::NonceTooHigh)) => {
                    Ok(())
                }
                Err(err) if err.to_string().contains("InsufficientLiquidity") => Ok(()),
                Err(err) => Err(err),
            }
        })
        .await?
    }

    /// Runs base-evm2's verified EIP-8130 preparation against the latest state and discards it.
    ///
    /// Sender and payer authorization, account changes, replay checks and payer rules fail at
    /// admission instead of after entering the pool; no phase calls run. A sequence ahead of its
    /// channel nonce is the sole tolerated inclusion error: the pool queues it until the gap is
    /// filled.
    #[cfg(feature = "base")]
    async fn validate_eip8130_pool_transaction(
        &self,
        pending: &PendingTransaction<FoundryTxEnvelope>,
        evm_env: &EvmEnv,
        ready_for_execution: bool,
    ) -> Result<(), BlockchainError> {
        let tx = PreparedTx::signed(pending.transaction.as_ref().clone(), *pending.sender());
        let evm_env = evm_env.clone();
        let opts = self.evm_options(&evm_env);
        self.with_database_at(None, move |state, _| {
            let mut evm = build_evm::<BaseEvmNetwork>(&*state, &evm_env, &opts)?;
            let evm_tx = BaseEvmNetwork::prepared_evm_tx(&tx, opts.cheats.as_ref())?;
            BaseEvmNetwork::set_tx_context(&mut evm, &evm_tx, &TxContext::default());
            match evm.validate_tx(&evm_tx).map_err(BlockchainError::from) {
                Ok(()) => Ok(()),
                Err(BlockchainError::InvalidTransaction(InvalidTransactionError::NonceTooHigh))
                    if !ready_for_execution =>
                {
                    Ok(())
                }
                Err(err)
                    if !ready_for_execution
                        && err.to_string().contains("is ahead of the channel nonce") =>
                {
                    Ok(())
                }
                Err(BlockchainError::InvalidTransaction(InvalidTransactionError::Rejected(
                    reason,
                ))) => Err(BlockchainError::Eip8130TransactionRejected(reason)),
                Err(BlockchainError::InvalidTransaction(err)) => {
                    Err(BlockchainError::Eip8130TransactionRejected(err.to_string()))
                }
                Err(err) => Err(err),
            }
        })
        .await?
    }
}

#[async_trait::async_trait]
impl<N: Network> TransactionValidator<FoundryTxEnvelope> for Backend<N>
where
    N: Network<TxEnvelope = FoundryTxEnvelope, ReceiptEnvelope = FoundryReceiptEnvelope>,
{
    async fn validate_pool_transaction(
        &self,
        tx: &PendingTransaction<FoundryTxEnvelope>,
    ) -> Result<(), BlockchainError> {
        let address = *tx.sender();
        let account = self.get_account(address).await?;
        #[cfg_attr(not(feature = "base"), allow(unused_mut))]
        let mut evm_env = self.next_evm_env(&**self.db.read().await)?;
        #[cfg(feature = "base")]
        if tx.transaction.as_ref().is_eip8130() {
            evm_env.block_env.timestamp = U256::from(self.eip8130_pool_timestamp());
        }

        #[cfg(feature = "base")]
        if let FoundryTxEnvelope::Eip8130(signed) = tx.transaction.as_ref() {
            let body = signed.tx();
            let now = self.eip8130_pool_timestamp_ms();
            signed
                .validate_admission_static(self.chain_id().to())
                .map_err(|error| BlockchainError::Eip8130TransactionRejected(error.to_string()))?;
            signed
                .validate_timestamp(now)
                .map_err(|error| BlockchainError::Eip8130TransactionRejected(error.to_string()))?;

            let rejected = |error: &dyn std::fmt::Display| {
                BlockchainError::Eip8130TransactionRejected(error.to_string())
            };

            let channel_nonce = if body.nonce_key == Eip8130Constants::NONCE_KEY_MAX {
                None
            } else if body.nonce_key.is_zero() {
                Some(account.nonce)
            } else {
                let slot = NonceManagerSlots::nonce_slot(address, body.nonce_key)
                    .ok_or_else(|| rejected(&"nonce_key 0 is the protocol nonce"))?;
                let word = self.storage_at(NonceManagerSlots::ADDRESS, slot, None).await?;
                Some(Eip8130Nonce::decode_channel_nonce(word.into()).to::<u64>())
            };
            if let Some(channel) = channel_nonce
                && body.nonce_sequence < channel
            {
                return Err(rejected(&format!(
                    "nonce sequence {} is below the channel nonce {channel}",
                    body.nonce_sequence
                )));
            }
            let nonce_key_first_use = channel_nonce == Some(0);
            let ready_for_execution =
                channel_nonce.is_none_or(|nonce| body.nonce_sequence == nonce);

            let intrinsic = IntrinsicGas::compute(
                signed,
                &signed.encoded_2718(),
                &IntrinsicGasInput::worst_case(
                    nonce_key_first_use,
                    IntrinsicGasInput::sender_auto_delegated(&body.account_changes),
                    body.payer.is_some(),
                ),
            )
            .map_err(|error| rejected(&error))?;
            if intrinsic.execution_gas_available(body.gas_limit).is_none() {
                return Err(rejected(&"sender intrinsic gas exceeds transaction gas limit"));
            }

            let payer = body.payer.unwrap_or(address);
            let payer_balance = if payer == address {
                account.balance
            } else {
                self.get_account(payer).await?.balance
            };
            let required = base::eip8130_max_fee_charge(
                body.gas_limit,
                intrinsic.payer_auth,
                body.max_fee_per_gas,
            );
            if payer_balance < required {
                return Err(rejected(&format!(
                    "gas payer balance {payer_balance} is below the required {required}"
                )));
            }

            self.validate_eip8130_pool_transaction(tx, &evm_env, ready_for_execution).await?;
        }

        // Tempo AA: validate time bounds (async checks)
        if let FoundryTxEnvelope::Tempo(aa_tx) = tx.transaction.as_ref() {
            let tempo_tx = aa_tx.tx();
            let current_time = evm_env.block_env.timestamp.saturating_to::<u64>();

            // Reject if valid_before is expired or too close to current time (< 3 seconds)
            const AA_VALID_BEFORE_MIN_SECS: u64 = 3;
            if let Some(valid_before) = tempo_tx.valid_before.map(|v| v.get()) {
                let min_allowed = current_time.saturating_add(AA_VALID_BEFORE_MIN_SECS);
                if valid_before <= min_allowed {
                    return Err(InvalidTransactionError::TempoValidBeforeExpired {
                        valid_before,
                        min_allowed,
                    }
                    .into());
                }

                let hardfork = self.tempo_hardfork();
                if hardfork.is_t1() && tempo_tx.is_expiring_nonce_tx() {
                    let max_expiry_secs = hardfork.expiring_nonce_max_expiry_secs();
                    let max_allowed = current_time.saturating_add(max_expiry_secs);
                    if valid_before > max_allowed {
                        return Err(InvalidTransactionError::TempoValidBeforeTooFar {
                            valid_before,
                            max_expiry_secs,
                            max_allowed,
                        }
                        .into());
                    }
                }
            }

            // Reject if valid_after is too far in the future. Mirrors Tempo's default pool limit
            // (`DEFAULT_AA_VALID_AFTER_MAX_SECS`), which is aligned with its queued transaction
            // lifetime.
            const AA_VALID_AFTER_MAX_SECS: u64 = 120;
            if let Some(valid_after) = tempo_tx.valid_after.map(|v| v.get()) {
                let max_allowed = current_time.saturating_add(AA_VALID_AFTER_MAX_SECS);
                if valid_after > max_allowed {
                    return Err(InvalidTransactionError::TempoValidAfterTooFar {
                        valid_after,
                        max_valid_after_secs: AA_VALID_AFTER_MAX_SECS,
                        max_allowed,
                    }
                    .into());
                }
            }
        }

        self.validate_pool_transaction_for(tx, &account, &evm_env)?;

        // Tempo charges gas in a fee token for every transaction type, never in the native token,
        // so the node's own pool validation replaces the native balance check skipped above.
        if self.is_tempo() && !self.disable_pool_balance_checks {
            self.validate_tempo_pool_transaction(tx, &evm_env).await?;
        }

        Ok(())
    }

    fn validate_pool_transaction_for(
        &self,
        pending: &PendingTransaction<FoundryTxEnvelope>,
        account: &AccountInfo,
        evm_env: &EvmEnv,
    ) -> Result<(), InvalidTransactionError> {
        let tx = &pending.transaction;

        if let Some(tx_chain_id) = tx.chain_id() {
            let chain_id = self.chain_id();
            if chain_id.to::<u64>() != tx_chain_id {
                if let FoundryTxEnvelope::Legacy(tx) = tx.as_ref() {
                    // <https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md>
                    if evm_env.cfg_env.spec >= SpecId::SPURIOUS_DRAGON && tx.chain_id().is_none() {
                        debug!(target: "backend", ?chain_id, ?tx_chain_id, "incompatible EIP155-based V");
                        return Err(InvalidTransactionError::IncompatibleEIP155);
                    }
                } else {
                    debug!(target: "backend", ?chain_id, ?tx_chain_id, "invalid chain id");
                    return Err(InvalidTransactionError::InvalidChainId);
                }
            }
        }

        // Reject native value transfers on Tempo networks
        if self.is_tempo() && !tx.value().is_zero() {
            warn!(target: "backend", "[{:?}] native value transfer not allowed in Tempo mode", tx.hash());
            return Err(InvalidTransactionError::TempoNativeValueTransfer);
        }

        // Tempo AA T5: cap authorization list size
        if self.is_tempo_hardfork_active(TempoHardfork::T5)
            && let FoundryTxEnvelope::Tempo(aa_tx) = tx.as_ref()
        {
            const MAX_TEMPO_AUTHORIZATIONS: usize = 16;
            let auth_count = aa_tx.tx().tempo_authorization_list.len();
            if auth_count > MAX_TEMPO_AUTHORIZATIONS {
                warn!(target: "backend", "[{:?}] Tempo tx has too many authorizations: {}", tx.hash(), auth_count);
                return Err(InvalidTransactionError::TempoTooManyAuthorizations {
                    count: auth_count,
                    max: MAX_TEMPO_AUTHORIZATIONS,
                });
            }
        }

        #[cfg(feature = "base")]
        if let FoundryTxEnvelope::Eip8130(signed) = pending.transaction.as_ref() {
            signed
                .validate_admission_static(evm_env.cfg_env.chain_id)
                .map_err(|error| InvalidTransactionError::Eip8130(error.to_string()))?;
            signed
                .validate_timestamp(
                    evm_env.block_env.timestamp.saturating_to::<u64>().saturating_mul(1_000),
                )
                .map_err(|error| InvalidTransactionError::Eip8130(error.to_string()))?;
        }

        // Nonce validation — skip for deposits (L1→L2), EIP-8130, and Tempo.
        #[cfg(any(feature = "base", feature = "optimism"))]
        let is_deposit_tx = pending.transaction.as_ref().is_deposit();
        #[cfg(not(any(feature = "base", feature = "optimism")))]
        let is_deposit_tx = false;
        #[cfg(feature = "base")]
        let is_eip8130_tx = pending.transaction.as_ref().is_eip8130();
        #[cfg(not(feature = "base"))]
        let is_eip8130_tx = false;
        let is_tempo_tx = pending.transaction.as_ref().is_tempo();
        let nonce = tx.nonce();
        if nonce < account.nonce && !is_deposit_tx && !is_eip8130_tx && !is_tempo_tx {
            debug!(target: "backend", "[{:?}] nonce too low", tx.hash());
            return Err(InvalidTransactionError::NonceTooLow);
        }

        #[cfg(feature = "monad")]
        self.validate_monad_transaction_type(tx)?;

        // EIP-4844 structural validation
        if evm_env.cfg_env.spec >= SpecId::CANCUN && tx.is_eip4844() {
            // Heavy (blob validation) checks
            let blob_tx = match tx.as_ref() {
                FoundryTxEnvelope::Eip4844(tx) => tx.tx(),
                _ => unreachable!(),
            };

            let blob_count = blob_tx.tx().blob_versioned_hashes.len();

            // Ensure there are blob hashes.
            if blob_count == 0 {
                return Err(InvalidTransactionError::NoBlobHashes);
            }

            // Ensure the tx does not exceed the max blobs per transaction.
            let max_blobs_per_tx = self.blob_params().max_blobs_per_tx as usize;
            if blob_count > max_blobs_per_tx {
                return Err(InvalidTransactionError::TooManyBlobs(blob_count, max_blobs_per_tx));
            }

            // Check for any blob validation errors if not impersonating.
            if !self.skip_blob_validation(Some(*pending.sender()))
                && let Err(err) = blob_tx.validate(EnvKzgSettings::default().get())
            {
                return Err(InvalidTransactionError::BlobTransactionValidationError(err));
            }
        }

        // EIP-3860 initcode size validation, respects --code-size-limit / --disable-code-size-limit
        if evm_env.cfg_env.spec >= SpecId::SHANGHAI && tx.kind().is_create() {
            let max_initcode_size = evm_env
                .cfg_env
                .limit_contract_code_size
                .map(|limit| limit.saturating_mul(2))
                .unwrap_or_else(|| self.max_initcode_size(evm_env));
            if tx.input().len() > max_initcode_size {
                return Err(InvalidTransactionError::MaxInitCodeSizeExceeded);
            }
        }

        // Balance and fee related checks
        if !self.disable_pool_balance_checks {
            // Gas limit validation. Under EIP-2780 a transaction's intrinsic gas starts at a lower
            // base, which execution then checks in full.
            let min_gas = if evm_env.cfg_env.spec >= SpecId::AMSTERDAM {
                EIP2780_TX_BASE_COST
            } else {
                MIN_TRANSACTION_GAS as u64
            };
            if tx.gas_limit() < min_gas {
                debug!(target: "backend", "[{:?}] gas too low", tx.hash());
                return Err(InvalidTransactionError::GasTooLow);
            }

            // Check tx gas limit against block gas limit, if block gas limit is set.
            if !evm_env.cfg_env.disable_block_gas_limit
                && tx.gas_limit() > evm_env.block_env.gas_limit
            {
                debug!(target: "backend", "[{:?}] gas too high", tx.hash());
                return Err(InvalidTransactionError::GasTooHigh(ErrDetail {
                    detail: String::from("tx.gas_limit > env.block.gas_limit"),
                }));
            }

            // Check tx gas limit against tx gas limit cap (Osaka hard fork and later).
            if let Some(cap) = self.enforced_tx_gas_limit_cap(evm_env)
                && tx.gas_limit() > cap
            {
                debug!(target: "backend", "[{:?}] gas too high", tx.hash());
                return Err(InvalidTransactionError::GasTooHigh(ErrDetail {
                    detail: String::from("tx.gas_limit > resolved tx gas limit cap"),
                }));
            }

            // EIP-1559 fee validation (London hard fork and later).
            if evm_env.cfg_env.spec >= SpecId::LONDON {
                if tx.max_fee_per_gas() < evm_env.block_env.basefee.into() && !is_deposit_tx {
                    debug!(target: "backend", "max fee per gas={}, too low, block basefee={}", tx.max_fee_per_gas(), evm_env.block_env.basefee);
                    return Err(InvalidTransactionError::FeeCapTooLow);
                }

                if !evm_env.cfg_env.disable_priority_fee_check
                    && let (Some(max_priority_fee_per_gas), max_fee_per_gas) =
                        (tx.as_ref().max_priority_fee_per_gas(), tx.as_ref().max_fee_per_gas())
                    && max_priority_fee_per_gas > max_fee_per_gas
                {
                    debug!(target: "backend", "max priority fee per gas={}, too high, max fee per gas={}", max_priority_fee_per_gas, max_fee_per_gas);
                    return Err(InvalidTransactionError::TipAboveFeeCap);
                }
            }

            // EIP-4844 blob fee validation
            if evm_env.cfg_env.spec >= SpecId::CANCUN
                && tx.is_eip4844()
                && let Some(max_fee_per_blob_gas) = tx.max_fee_per_blob_gas()
                && let Some(blob_gas_and_price) = &evm_env.block_env.blob_excess_gas_and_price
                && max_fee_per_blob_gas < blob_gas_and_price.blob_gasprice
            {
                debug!(target: "backend", "max fee per blob gas={}, too low, block blob gas price={}", max_fee_per_blob_gas, blob_gas_and_price.blob_gasprice);
                return Err(InvalidTransactionError::BlobFeeCapTooLow(
                    max_fee_per_blob_gas,
                    blob_gas_and_price.blob_gasprice,
                ));
            }

            let value = tx.value();
            match tx.as_ref() {
                #[cfg(any(feature = "base", feature = "optimism"))]
                FoundryTxEnvelope::Deposit(deposit_tx) => {
                    // Deposit transactions
                    // https://specs.optimism.io/protocol/deposits.html#execution
                    // 1. no gas cost check required since already have prepaid gas from L1
                    // 2. increment account balance by deposited amount before checking for
                    //    sufficient funds `tx.value <= existing account value + deposited value`
                    if value > account.balance + U256::from(deposit_tx.mint) {
                        debug!(target: "backend", "[{:?}] insufficient balance={}, required={} account={:?}", tx.hash(), account.balance + U256::from(deposit_tx.mint), value, *pending.sender());
                        return Err(InvalidTransactionError::InsufficientFunds);
                    }
                }
                #[cfg(feature = "base")]
                FoundryTxEnvelope::Eip8130(_) => {
                    // EIP-8130 affordability is checked asynchronously against
                    // the resolved payer, including payer-authentication gas.
                }
                FoundryTxEnvelope::Tempo(_) => {
                    // Tempo AA transactions pay gas with fee tokens, not ETH.
                    // Fee token balance is validated in validate_pool_transaction (async).
                }
                _ if self.is_tempo() => {
                    // Every transaction on Tempo pays gas in a fee token; forked mainnet senders
                    // hold no native balance at all. The fee token balance is validated in
                    // validate_pool_transaction (async).
                }
                #[cfg(feature = "monad")]
                _ if self.validate_monad_transaction_funds(pending, account, evm_env)? => {}
                _ => {
                    let max_cost = (tx.gas_limit() as u128)
                        .saturating_mul(tx.max_fee_per_gas())
                        .saturating_add(
                            tx.blob_gas_used()
                                .map(|g| g as u128)
                                .unwrap_or(0)
                                .mul(tx.max_fee_per_blob_gas().unwrap_or(0)),
                        );
                    // check sufficient funds: `gas * price + value`
                    let req_funds =
                        max_cost.checked_add(value.saturating_to()).ok_or_else(|| {
                            debug!(target: "backend", "[{:?}] cost too high", tx.hash());
                            InvalidTransactionError::InsufficientFunds
                        })?;
                    if account.balance < U256::from(req_funds) {
                        debug!(target: "backend", "[{:?}] insufficient balance={}, required={} account={:?}", tx.hash(), account.balance, req_funds, *pending.sender());
                        return Err(InvalidTransactionError::InsufficientFunds);
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_for(
        &self,
        tx: &PendingTransaction<FoundryTxEnvelope>,
        account: &AccountInfo,
        evm_env: &EvmEnv,
    ) -> Result<(), InvalidTransactionError> {
        self.validate_pool_transaction_for(tx, account, evm_env)?;
        // EIP-8130 counts nonces per channel in the nonce manager, so `tx.nonce()` is not
        // comparable to the account's protocol nonce. Admission already validated the channel.
        #[cfg(feature = "base")]
        if tx.transaction.as_ref().is_eip8130() {
            return Ok(());
        }
        if tx.nonce() > account.nonce {
            return Err(InvalidTransactionError::NonceTooHigh);
        }
        Ok(())
    }
}

/// Replaces the cached hash of a [`Signed`] transaction, preserving the inner tx and signature.
fn rehash<T>(signed: Signed<T>, hash: B256) -> Signed<T>
where
    T: alloy_consensus::transaction::RlpEcdsaEncodableTx,
{
    let (t, sig, _) = signed.into_parts();
    Signed::new_unchecked(t, sig, hash)
}

fn build_rpc_transaction(
    envelope: AnyTxEnvelope,
    from: Address,
    block: Option<&Block>,
    info: Option<&TransactionInfo>,
    effective_gas_price: Option<u128>,
) -> AnyRpcTransaction {
    let tx = Transaction {
        inner: Recovered::new_unchecked(envelope, from),
        block_hash: block.map(|block| block.header.hash_slow()),
        block_number: block.map(|block| block.header.number()),
        transaction_index: info.map(|info| info.transaction_index),
        effective_gas_price,
        block_timestamp: block.map(|block| block.header.timestamp()),
    };
    AnyRpcTransaction::from(WithOtherFields::new(tx))
}

/// Creates a `AnyRpcTransaction` as it's expected for the `eth` RPC api from storage data
pub fn transaction_build(
    tx_hash: Option<B256>,
    eth_transaction: MaybeImpersonatedTransaction<FoundryTxEnvelope>,
    block: Option<&Block>,
    info: Option<TransactionInfo>,
    base_fee: Option<u64>,
) -> AnyRpcTransaction {
    let mined_from = info.as_ref().map(|info| info.from);

    #[cfg(any(feature = "base", feature = "optimism"))]
    if let FoundryTxEnvelope::Deposit(deposit_tx) = eth_transaction.as_ref() {
        let dep_tx = deposit_tx;

        let ser = serde_json::to_value(dep_tx).expect("could not serialize TxDeposit");
        let maybe_deposit_fields = OtherFields::try_from(ser);

        match maybe_deposit_fields {
            Ok(mut fields) => {
                fields.remove("hash");
                fields.remove("from");
                // Add zeroed signature fields for backwards compatibility
                // https://specs.optimism.io/protocol/deposits.html#the-deposited-transaction-type
                fields.insert("v".to_string(), serde_json::to_value("0x0").unwrap());
                fields.insert("r".to_string(), serde_json::to_value(B256::ZERO).unwrap());
                fields.insert(String::from("s"), serde_json::to_value(B256::ZERO).unwrap());
                fields.insert(String::from("nonce"), serde_json::to_value("0x0").unwrap());

                let inner = UnknownTypedTransaction {
                    ty: AnyTxType(DEPOSIT_TX_TYPE_ID),
                    fields,
                    memo: Default::default(),
                };

                let envelope = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
                    hash: tx_hash.unwrap_or_else(|| eth_transaction.hash()),
                    inner,
                });

                return build_rpc_transaction(
                    envelope,
                    mined_from.unwrap_or(deposit_tx.from),
                    block,
                    info.as_ref(),
                    None,
                );
            }
            Err(_) => {
                error!(target: "backend", "failed to serialize deposit transaction");
            }
        }
    }

    #[cfg(feature = "base")]
    if let FoundryTxEnvelope::Eip8130(eip8130_tx) = eth_transaction.as_ref() {
        let from = mined_from.unwrap_or_else(|| eth_transaction.recover().unwrap_or_default());
        let rpc_tx = BaseRpcTransaction::from_transaction(
            Recovered::new_unchecked(BaseTxEnvelope::Eip8130(eip8130_tx.clone()), from),
            BaseTransactionInfo::default(),
        );
        let mut fields = OtherFields::try_from(
            serde_json::to_value(rpc_tx)
                .expect("could not serialize canonical EIP-8130 transaction"),
        )
        .expect("EIP-8130 transaction must serialize to an object");
        for key in [
            "hash",
            "type",
            "from",
            "blockHash",
            "blockNumber",
            "transactionIndex",
            "effectiveGasPrice",
        ] {
            fields.remove(key);
        }
        let envelope = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
            hash: eth_transaction.hash(),
            inner: UnknownTypedTransaction {
                ty: AnyTxType(base_common_consensus::EIP8130_TX_TYPE_ID),
                fields,
                memo: Default::default(),
            },
        });
        return build_rpc_transaction(envelope, from, block, info.as_ref(), None);
    }

    if let FoundryTxEnvelope::Celo(tx) = eth_transaction.as_ref() {
        let mut fields =
            OtherFields::try_from(serde_json::to_value(tx).expect("valid CIP-64 transaction"))
                .expect("CIP-64 transaction serializes to an object");
        fields.remove("hash");
        let envelope = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
            hash: tx_hash.unwrap_or_else(|| eth_transaction.hash()),
            inner: UnknownTypedTransaction {
                ty: AnyTxType(foundry_primitives::CIP64_TX_TYPE),
                fields,
                memo: Default::default(),
            },
        });
        let from = mined_from.unwrap_or_else(|| eth_transaction.recover().unwrap_or_default());
        let effective_gas_price = block.map(|_| eth_transaction.effective_gas_price(base_fee));
        return build_rpc_transaction(envelope, from, block, info.as_ref(), effective_gas_price);
    }
    if let FoundryTxEnvelope::Tempo(tempo_tx) = eth_transaction.as_ref() {
        let from = mined_from.unwrap_or_else(|| eth_transaction.recover().unwrap_or_default());
        let ser = serde_json::to_value(tempo_tx).expect("could not serialize Tempo transaction");
        let maybe_tempo_fields = OtherFields::try_from(ser);

        match maybe_tempo_fields {
            Ok(mut fields) => {
                fields.remove("hash");
                let inner = UnknownTypedTransaction {
                    ty: AnyTxType(TEMPO_TX_TYPE_ID),
                    fields,
                    memo: Default::default(),
                };

                let envelope = AnyTxEnvelope::Unknown(UnknownTxEnvelope {
                    hash: tx_hash.unwrap_or_else(|| eth_transaction.hash()),
                    inner,
                });

                return build_rpc_transaction(envelope, from, block, info.as_ref(), None);
            }
            Err(_) => {
                error!(target: "backend", "failed to serialize tempo transaction");
            }
        }
    }

    let from = mined_from.unwrap_or_else(|| eth_transaction.recover().unwrap_or_default());
    let effective_gas_price = eth_transaction.effective_gas_price(base_fee);

    // if a specific hash was provided we update the transaction's hash
    // This is important for impersonated transactions since they all use the
    // `BYPASS_SIGNATURE` which would result in different hashes
    // Note: for impersonated transactions this only concerns pending transactions because
    // there's no `info` yet.
    let hash = tx_hash.unwrap_or_else(|| eth_transaction.hash());

    let eth_envelope = FoundryTxEnvelope::from(eth_transaction)
        .try_into_eth()
        .expect("non-standard transactions are handled above");

    let envelope = match eth_envelope {
        TxEnvelope::Legacy(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Legacy(rehash(s, hash))),
        TxEnvelope::Eip1559(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Eip1559(rehash(s, hash))),
        TxEnvelope::Eip2930(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Eip2930(rehash(s, hash))),
        TxEnvelope::Eip4844(s) => {
            let s = if block.is_some() { s.map(TxEip4844Variant::drop_sidecar) } else { s };
            AnyTxEnvelope::Ethereum(TxEnvelope::Eip4844(rehash(s, hash)))
        }
        TxEnvelope::Eip7702(s) => AnyTxEnvelope::Ethereum(TxEnvelope::Eip7702(rehash(s, hash))),
    };

    build_rpc_transaction(envelope, from, block, info.as_ref(), Some(effective_gas_price))
}

/// Prove a storage key's existence or nonexistence in the account's storage trie.
///
/// `storage_key` is the hash of the desired storage key, meaning
/// this will only work correctly under a secure trie.
/// `storage_key` == keccak(key)
pub fn prove_storage(
    storage: &alloy_primitives::map::U256Map<U256>,
    keys: &[B256],
) -> (B256, Vec<Vec<Bytes>>) {
    let keys: Vec<_> = keys.iter().map(|key| Nibbles::unpack(keccak256(key))).collect();

    let mut builder = HashBuilder::default().with_proof_retainer(ProofRetainer::new(keys.clone()));

    for (key, value) in trie_storage(storage) {
        builder.add_leaf(key, &value);
    }

    let root = builder.root();

    let mut proofs = Vec::new();
    let all_proof_nodes = builder.take_proof_nodes();

    for proof_key in keys {
        // Iterate over all proof nodes and find the matching ones.
        // The filtered results are guaranteed to be in order.
        let matching_proof_nodes =
            all_proof_nodes.matching_nodes_sorted(&proof_key).into_iter().map(|(_, node)| node);
        proofs.push(matching_proof_nodes.collect());
    }

    (root, proofs)
}

/// Builds an account and storage proof from one fixed local account set.
fn prove_account(db: &AddressMap<DbAccount>, address: Address, keys: &[B256]) -> AccountProof {
    let default = DbAccount::default();
    let account = db.get(&address).unwrap_or(&default);

    let mut builder = HashBuilder::default()
        .with_proof_retainer(ProofRetainer::new(vec![Nibbles::unpack(keccak256(address))]));

    for (key, account) in trie_accounts(db) {
        builder.add_leaf(key, &account);
    }

    let _ = builder.root();

    let proof =
        builder.take_proof_nodes().into_nodes_sorted().into_iter().map(|(_, v)| v).collect();
    let (storage_hash, storage_proofs) = prove_storage(&account.storage, keys);

    AccountProof {
        address,
        balance: account.info.balance,
        nonce: account.info.nonce,
        code_hash: account.info.code_hash,
        storage_hash,
        account_proof: proof,
        storage_proof: keys
            .iter()
            .copied()
            .zip(storage_proofs)
            .map(|(key, proof)| {
                let storage_key: U256 = key.into();
                let value = account.storage.get(&storage_key).copied().unwrap_or_default();
                StorageProof { key: JsonStorageKey::Hash(key), value, proof }
            })
            .collect(),
    }
}

pub fn is_arbitrum(chain_id: u64) -> bool {
    if let Ok(chain) = NamedChain::try_from(chain_id) {
        return chain.is_arbitrum();
    }
    false
}

fn simulate_rpc_error(code: i64, message: impl Into<String>) -> BlockchainError {
    BlockchainError::RpcError(RpcError {
        code: ErrorCode::from(code),
        message: message.into().into(),
        data: None,
    })
}

fn previously_deleted_accounts(
    accounts: &AddressMap<DbAccount>,
    addresses: impl IntoIterator<Item = Address>,
) -> Vec<Address> {
    addresses
        .into_iter()
        .filter(|address| {
            accounts
                .get(address)
                .is_some_and(|account| account.account_state == AccountState::NotExisting)
        })
        .collect()
}

fn preserve_deleted_storage(
    accounts: &mut AddressMap<DbAccount>,
    previously_deleted: Vec<Address>,
) {
    for address in previously_deleted {
        if let Some(account) = accounts.get_mut(&address)
            && account.account_state != AccountState::NotExisting
        {
            account.account_state = AccountState::StorageCleared;
        }
    }
}

fn simulate_transaction_error(error: InvalidTransactionError) -> BlockchainError {
    let code = match &error {
        InvalidTransactionError::NonceTooLow => -38010,
        InvalidTransactionError::NonceTooHigh => -38011,
        InvalidTransactionError::NonceMaxValue => -32603,
        InvalidTransactionError::FeeCapTooLow => -38012,
        InvalidTransactionError::GasTooLow | InvalidTransactionError::GasTooHigh(_) => -38013,
        InvalidTransactionError::InsufficientFunds
        | InvalidTransactionError::InsufficientFundsForTransfer => -38014,
        _ => return BlockchainError::InvalidTransaction(error),
    };

    simulate_rpc_error(code, format!("err: {error}"))
}

pub(in crate::eth) fn sanitize_simulation_blocks<T>(
    blocks: Vec<SimBlock<T>>,
    base_number: u64,
    base_timestamp: u64,
    block_interval: u64,
) -> Result<Vec<SimBlock<T>>, BlockchainError> {
    let block_interval = block_interval.max(1);
    let mut sanitized = Vec::with_capacity(blocks.len());
    let mut previous_number = base_number;
    let mut previous_timestamp = base_timestamp;

    for mut block in blocks {
        let mut overrides = block.block_overrides.take().unwrap_or_default();
        let default_number = previous_number.checked_add(1).ok_or_else(|| {
            simulate_rpc_error(-38020, "block number overflow while constructing sequence")
        })?;
        let number =
            overrides.number.map(|number| number.saturating_to()).unwrap_or(default_number);

        if number <= previous_number {
            return Err(simulate_rpc_error(
                -38020,
                format!("block numbers must be in order: {number} <= {previous_number}"),
            ));
        }

        let gap = number - previous_number - 1;
        let remaining = MAX_SIMULATE_BLOCKS as usize - sanitized.len();
        if gap as usize >= remaining {
            return Err(simulate_rpc_error(-38026, "too many blocks"));
        }

        for offset in 0..gap {
            let timestamp = previous_timestamp.checked_add(block_interval).ok_or_else(|| {
                simulate_rpc_error(-38021, "block timestamp overflow while filling number gap")
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
            None => previous_timestamp.checked_add(block_interval).ok_or_else(|| {
                simulate_rpc_error(-38021, "block timestamp overflow while constructing sequence")
            })?,
        };
        if timestamp <= previous_timestamp {
            return Err(simulate_rpc_error(
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

fn arbitrum_replay_block_number(block: &AnyRpcBlock) -> U256 {
    block
        .other
        .get("l1BlockNumber")
        .cloned()
        .and_then(|number| serde_json::from_value(number).ok())
        .unwrap_or_else(|| U256::from(block.header().number()))
}

/// Creates an Ethereum-shaped genesis header from the EVM environment.
fn genesis_header(
    evm_env: &EvmEnv,
    base_fee: Option<u64>,
    timestamp: u64,
    genesis_number: u64,
) -> Header {
    let spec_id = *evm_env.spec_id();
    Header {
        timestamp,
        base_fee_per_gas: base_fee,
        gas_limit: evm_env.block_env.gas_limit,
        beneficiary: evm_env.block_env.beneficiary,
        difficulty: evm_env.block_env.difficulty,
        blob_gas_used: evm_env.block_env.blob_excess_gas_and_price.as_ref().map(|_| 0),
        excess_blob_gas: evm_env.block_env.blob_excess_gas(),
        number: genesis_number,
        parent_beacon_block_root: (spec_id >= SpecId::CANCUN).then_some(Default::default()),
        withdrawals_root: (spec_id >= SpecId::SHANGHAI).then_some(EMPTY_WITHDRAWALS),
        requests_hash: (spec_id >= SpecId::PRAGUE).then_some(EMPTY_REQUESTS_HASH),
        ..Default::default()
    }
}

/// Creates an Ethereum header from a `genesis.json` configuration.
fn genesis_json_header(genesis: &Genesis) -> Header {
    let number = genesis.number.unwrap_or_default();
    let timestamp = genesis.timestamp;
    let is_london = genesis.config.is_london_active_at_block(number);
    let is_shanghai = genesis.config.is_shanghai_active_at_block_and_timestamp(number, timestamp);
    let is_cancun = genesis.config.is_cancun_active_at_block_and_timestamp(number, timestamp);
    let is_prague = genesis.config.prague_time.is_some_and(|fork| fork <= timestamp);
    let is_amsterdam = genesis.config.amsterdam_time.is_some_and(|fork| fork <= timestamp);

    Header {
        number,
        parent_hash: genesis.parent_hash.unwrap_or_default(),
        gas_limit: genesis.gas_limit,
        difficulty: genesis.difficulty,
        nonce: genesis.nonce.into(),
        extra_data: genesis.extra_data.clone(),
        state_root: state_root_ref_unhashed(&genesis.alloc),
        timestamp,
        mix_hash: genesis.mix_hash,
        beneficiary: genesis.coinbase,
        base_fee_per_gas: is_london.then(|| {
            genesis
                .base_fee_per_gas
                .map(|fee| fee as u64)
                .unwrap_or(crate::eth::fees::INITIAL_BASE_FEE)
        }),
        withdrawals_root: is_shanghai.then_some(EMPTY_WITHDRAWALS),
        parent_beacon_block_root: is_cancun.then_some(B256::ZERO),
        blob_gas_used: is_cancun.then_some(genesis.blob_gas_used.unwrap_or_default()),
        excess_blob_gas: is_cancun.then_some(genesis.excess_blob_gas.unwrap_or_default()),
        requests_hash: is_prague.then_some(EMPTY_REQUESTS_HASH),
        block_access_list_hash: is_amsterdam.then_some(EMPTY_BLOCK_ACCESS_LIST_HASH),
        slot_number: is_amsterdam.then_some(genesis.slot_number.unwrap_or_default()),
        ..Default::default()
    }
}

/// Wraps an Ethereum-shaped header in the selected network's consensus header.
fn foundry_header(networks: &NetworkConfigs, header: Header) -> FoundryHeader {
    if networks.is_tempo() { FoundryHeader::tempo(header) } else { header.into() }
}

#[cfg(test)]
mod tests {
    use super::{
        BlockRequest, CallOutcome, FeeDetails, ForkCacheNamespace, ForkCacheSource, InstrStop,
        MiningCommitHook, StagedForkCacheLease, StagedForkDbUser, arbitrum_replay_block_number,
    };
    use crate::{
        NodeConfig,
        config::ForkTransactionReplay,
        eth::backend::db::{BlockchainDb, BlockchainDbMeta},
        spawn,
    };
    use alloy_network::{AnyHeader, AnyRpcBlock, AnyRpcHeader, TransactionBuilder};
    use alloy_primitives::{Address, B256, Bytes, U256};
    use alloy_provider::Provider;
    use alloy_rpc_types::{Block, BlockTransactions, TransactionRequest, state::EvmOverrides};
    use alloy_serde::WithOtherFields;
    use foundry_config::NamedChain;
    use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork};
    use foundry_evm_networks::arbitrum;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::Notify;

    #[cfg(feature = "base")]
    use foundry_evm_hardforks::BaseUpgrade;
    #[cfg(feature = "base")]
    use foundry_evm_traces::CallTraceDecoderBuilder;

    #[cfg(all(feature = "base", feature = "optimism"))]
    use alloy_consensus::Header;
    #[cfg(all(feature = "base", feature = "optimism"))]
    use foundry_evm_hardforks::OpHardfork;
    #[cfg(all(feature = "base", feature = "optimism"))]
    use foundry_evm_networks::NetworkConfigs;

    fn test_cache_db(cache_path: std::path::PathBuf) -> BlockchainDb {
        let db = BlockchainDb::new(BlockchainDbMeta::default(), Some(cache_path));
        db.block_hashes().write().insert(U256::ZERO, B256::repeat_byte(0x11));
        db.cache().flush();
        db
    }

    #[test]
    fn arbitrum_transaction_replay_uses_l1_block_number() {
        let header = AnyHeader { number: 75_219_831, ..Default::default() };
        let mut block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                BlockTransactions::Full(Vec::new()),
            )
            .into(),
        );
        block.other.insert("l1BlockNumber".to_string(), serde_json::json!("0x10276d3"));

        assert_eq!(arbitrum_replay_block_number(&block), U256::from(16_938_707));
    }

    #[cfg(all(feature = "base", feature = "optimism"))]
    #[tokio::test(flavor = "multi_thread")]
    async fn optimism_fork_uses_optimism_schedule_for_base_headers() {
        let timestamp = 1_710_374_401;
        let (_, origin) = spawn(
            NodeConfig::test_base()
                .with_hardfork(Some(BaseUpgrade::Ecotone.into()))
                .with_genesis_timestamp(Some(timestamp)),
        )
        .await;
        let (api, _) = spawn(
            NodeConfig::test()
                .with_networks(NetworkConfigs::with_optimism())
                .with_eth_rpc_url(Some(origin.http_endpoint())),
        )
        .await;

        assert!(api.backend.is_optimism());
        assert_eq!(api.backend.hardfork(), OpHardfork::Ecotone.into());
        assert!(
            !api.backend
                .is_optimism_jovian_at_header(&Header { timestamp, ..Default::default() }, None)
        );
    }

    #[tokio::test]
    async fn fork_arbitrum_transaction_replay_preserves_rpc_block_number() {
        const GENESIS_BLOCK: u64 = 101;
        const L1_BLOCK: u64 = 10;
        const REPLAY_BLOCK: u64 = GENESIS_BLOCK + 1;

        let config = || {
            NodeConfig::test()
                .with_chain_id(Some(NamedChain::Arbitrum as u64))
                .with_genesis_block_number(Some(GENESIS_BLOCK))
        };
        let (_source_api, source_handle) = spawn(config()).await;
        let source_provider = source_handle.http_provider();
        let sender = source_provider.get_accounts().await.unwrap()[0];
        let receipt = source_provider
            .send_transaction(WithOtherFields::new(
                TransactionRequest::default()
                    .with_from(sender)
                    .with_to(arbitrum::ARB_SYS_ADDRESS)
                    .with_input(Bytes::from(arbitrum::ARB_BLOCK_NUMBER_SELECTOR)),
            ))
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        let mut source_block = source_provider
            .get_block_by_hash(receipt.block_hash.unwrap())
            .full()
            .await
            .unwrap()
            .unwrap();
        source_block
            .other
            .insert("l1BlockNumber".to_string(), serde_json::json!(format!("0x{L1_BLOCK:x}")));

        let (replay_api, _replay_handle) = spawn(config()).await;
        replay_api
            .backend
            .apply_fork_transaction_replay(ForkTransactionReplay { source_block, target_index: 0 })
            .await
            .unwrap();

        let expected = arbitrum::arb_block_number_output(REPLAY_BLOCK);
        let replayed =
            replay_api.backend.mined_transaction_receipt(receipt.transaction_hash).unwrap();
        assert_eq!(replayed.out.unwrap(), expected);
        assert_eq!(replay_api.block_number().unwrap(), U256::from(REPLAY_BLOCK));

        let output = replay_api
            .call(
                WithOtherFields::new(
                    TransactionRequest::default()
                        .with_to(arbitrum::ARB_SYS_ADDRESS)
                        .with_input(Bytes::from(arbitrum::ARB_BLOCK_NUMBER_SELECTOR)),
                ),
                None,
                EvmOverrides::default(),
            )
            .await
            .unwrap();
        assert_eq!(output, expected);
    }

    fn test_endpoint_identity(
        hardfork: Option<FoundryHardfork>,
        instance_id: Option<B256>,
    ) -> super::ForkEndpointIdentity {
        super::ForkEndpointIdentity {
            execution_chain_id: 1,
            source_chain_id: 1,
            network: None,
            network_profile: None,
            hardfork,
            blob_params: None,
            instance_id,
            source_fork_block_number: None,
            source_fork_block_hash: None,
        }
    }

    #[tokio::test]
    async fn missing_snapshot_block_does_not_change_head() {
        let (api, _handle) = spawn(NodeConfig::test()).await;
        let snapshot_hash = api.backend.best_hash();
        let snapshot = api.backend.create_state_snapshot().await;
        api.mine_one().await.unwrap();
        let best_number = api.backend.best_number();
        let best_hash = api.backend.best_hash();
        let head_wall_time = api.backend.time().last_block_wall_time();

        api.backend.blockchain.storage.write().blocks.remove(&snapshot_hash);
        let err = api.backend.revert_state_snapshot(snapshot).await.unwrap_err();

        assert!(matches!(err, super::BlockchainError::BlockNotFound));
        assert_eq!(api.backend.best_number(), best_number);
        assert_eq!(api.backend.best_hash(), best_hash);
        assert_eq!(api.backend.time().last_block_wall_time(), head_wall_time);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn live_execution_snapshot_is_coherent_during_mining() {
        let (api, handle) = spawn(NodeConfig::test().with_no_mining(true)).await;
        let sender = handle.dev_wallets().next().unwrap().address();
        let recipient = Address::repeat_byte(0x11);
        let contract = Address::repeat_byte(0x22);

        // Return the recipient balance followed by NUMBER. At block 0 both are zero; at block 1,
        // after the queued transfer is mined, both are one.
        let mut code = vec![0x73];
        code.extend_from_slice(recipient.as_slice());
        code.extend_from_slice(&[
            0x31, 0x60, 0x00, 0x52, 0x43, 0x60, 0x20, 0x52, 0x60, 0x40, 0x60, 0x00, 0xf3,
        ]);
        api.anvil_set_code(contract, code.into()).await.unwrap();
        api.send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(sender).to(recipient).value(U256::ONE),
        ))
        .await
        .unwrap();

        let resolved_head = api.backend.best_number();
        let db_guard = api.backend.db.write().await;

        // Polling while holding the database makes the lock queue deterministic: mining owns the
        // first waiter and the call that already resolved `latest` owns the second.
        let mining_api = api.clone();
        let mut mining = Box::pin(async move { mining_api.mine_one().await });
        assert!(futures::poll!(mining.as_mut()).is_pending());

        let request = WithOtherFields::new(TransactionRequest::default().to(contract));
        let mut call = Box::pin(api.backend.call(
            request,
            FeeDetails::zero(),
            Some(BlockRequest::Number(resolved_head)),
            EvmOverrides::default(),
        ));
        assert!(futures::poll!(call.as_mut()).is_pending());

        let mut pending_block = Box::pin(api.backend.pending_block(Vec::new()));
        assert!(futures::poll!(pending_block.as_mut()).is_pending());

        let mut state = Box::pin(api.serialized_state(false));
        assert!(futures::poll!(state.as_mut()).is_pending());

        // Pause mining after the database commit but before canonical publication without locking
        // storage, so an incorrectly unblocked pending reader can observe the old parent.
        let hook =
            MiningCommitHook { reached: Arc::new(Notify::new()), resume: Arc::new(Notify::new()) };
        *api.backend.mining_commit_hook.lock() = Some(hook.clone());
        drop(db_guard);
        let mining = tokio::spawn(mining);
        hook.reached.notified().await;

        // Mining retains the database write lock while waiting to publish the canonical head, so
        // neither a live call nor a pending block can observe the partially published snapshot.
        assert!(futures::poll!(call.as_mut()).is_pending());
        assert!(futures::poll!(pending_block.as_mut()).is_pending());
        assert!(futures::poll!(state.as_mut()).is_pending());

        hook.resume.notify_one();
        mining.await.unwrap().unwrap();

        let CallOutcome { exit, output, .. } = call.await.unwrap();
        let pending_block = pending_block.await.unwrap();
        let state = state.await.unwrap();

        assert_eq!(state.accounts[&recipient].balance, U256::ONE);
        assert_eq!(state.block.unwrap().number, U256::ONE);
        assert_eq!(state.best_block_number, Some(1));

        assert_eq!(exit, InstrStop::Return);
        assert_eq!(output.len(), 64);
        let balance = U256::from_be_slice(&output[..32]);
        let block_number = U256::from_be_slice(&output[32..]);
        // The call requested block 0, so it sees block 0's balance and number.
        assert_eq!((balance, block_number), (U256::ZERO, U256::ZERO));

        assert_eq!(pending_block.block.header.number, api.backend.best_number() + 1);
        assert_eq!(pending_block.block.header.parent_hash, api.backend.best_hash());
        assert_eq!(pending_block.block.header.base_fee_per_gas, Some(875_175_000));
    }

    /// Reads the recipient balance at head block 0 while block 1, which funds it, is being mined.
    async fn read_head_balance_while_mining(config: NodeConfig) -> U256 {
        let (api, handle) = spawn(config.with_no_mining(true)).await;
        let sender = handle.dev_wallets().next().unwrap().address();
        let recipient = Address::repeat_byte(0x11);
        api.send_transaction(WithOtherFields::new(
            TransactionRequest::default().from(sender).to(recipient).value(U256::from(1)),
        ))
        .await
        .unwrap();

        // Pause mining after the next block's state is committed while it still holds the
        // database write lock and before it publishes the new head.
        let hook =
            MiningCommitHook { reached: Arc::new(Notify::new()), resume: Arc::new(Notify::new()) };
        *api.backend.mining_commit_hook.lock() = Some(hook.clone());
        let mining_api = api.clone();
        let mining = tokio::spawn(async move { mining_api.mine_one().await });
        hook.reached.notified().await;

        // The read names the current head, block 0, and queues behind the miner's write lock.
        let head = api.backend.best_number();
        assert_eq!(head, 0);
        let mut balance =
            Box::pin(api.backend.get_balance(recipient, Some(BlockRequest::Number(head))));
        assert!(futures::poll!(balance.as_mut()).is_pending());

        hook.resume.notify_one();
        mining.await.unwrap().unwrap();
        assert_eq!(api.backend.best_number(), 1);

        balance.await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn head_number_read_waiting_on_mining_keeps_requested_state() {
        // The read must be answered from block 0, not from block 1 mined while it waited.
        assert_eq!(read_head_balance_while_mining(NodeConfig::test()).await, U256::ZERO);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn head_number_read_waiting_on_mining_without_history_reads_live_state() {
        // Without state history block 0's state is gone, so the read is served from the live state
        // instead of failing.
        let config = NodeConfig::test().set_pruned_history(Some(None));
        assert_eq!(read_head_balance_while_mining(config).await, U256::from(1));
    }

    struct CacheFlushingDb(BlockchainDb);

    impl Drop for CacheFlushingDb {
        fn drop(&mut self) {
            self.0.block_hashes().write().insert(U256::ONE, B256::repeat_byte(0x22));
            self.0.cache().flush();
        }
    }

    #[test]
    fn staged_fork_cache_lease_waits_for_last_owner() {
        let root = tempdir().unwrap();
        let block_cache_dir = root.path().join("1");
        let cache_path = block_cache_dir.join("storage.json");
        let sibling_cache_path = block_cache_dir.join("storage-sibling.json");
        let db = test_cache_db(cache_path.clone());
        std::fs::write(&sibling_cache_path, b"sibling").unwrap();
        let lease = StagedForkCacheLease::new(db.clone(), Some(cache_path.clone()));
        let last_user = lease.clone();

        lease.rollback().unwrap();
        assert!(!db.block_hashes().read().is_empty());
        drop(lease);
        assert!(!db.block_hashes().read().is_empty());

        drop(last_user);
        assert!(db.block_hashes().read().is_empty());
        assert!(!cache_path.exists());
        assert!(sibling_cache_path.exists());
    }

    #[tokio::test]
    async fn staged_fork_cache_lease_survives_task_cancellation() {
        let root = tempdir().unwrap();
        let block_cache_dir = root.path().join("1");
        let cache_path = block_cache_dir.join("storage.json");
        let cache_db = test_cache_db(cache_path.clone());
        let lease = StagedForkCacheLease::new(cache_db.clone(), Some(cache_path.clone()));
        let db = Arc::new(tokio::sync::RwLock::new(CacheFlushingDb(cache_db.clone())));
        let task_user = StagedForkDbUser { db: Some(Arc::clone(&db)), cache_lease: lease.clone() };
        drop(db);
        let task = tokio::spawn(async move {
            let _db = task_user.db().read().await;
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;

        // Precise closure capture must not detach the lease from the task's database handle.
        assert_eq!(Arc::strong_count(lease.0.as_ref().unwrap()), 2);
        lease.rollback().unwrap();
        drop(lease);

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        assert!(cache_db.block_hashes().read().is_empty());
        assert!(!cache_path.exists());
    }

    #[test]
    fn staged_fork_cache_lease_disarm_preserves_committed_cache() {
        let root = tempdir().unwrap();
        let block_cache_dir = root.path().join("1");
        let cache_path = block_cache_dir.join("storage.json");
        let db = test_cache_db(cache_path.clone());
        let lease = StagedForkCacheLease::new(db.clone(), Some(cache_path.clone()));

        lease.disarm();
        drop(lease);

        assert!(!db.block_hashes().read().is_empty());
        assert!(cache_path.exists());
    }

    #[test]
    fn fork_cache_namespace_preserves_sibling_endpoints() {
        let root = tempdir().unwrap();
        let target_file = "storage-target.json";
        let sibling_file = "storage-sibling.json";
        let regular_cache_entry = root.path().join("latest.json");
        std::fs::write(&regular_cache_entry, b"ordinary RPC cache entry").unwrap();
        for block in ["1", "2"] {
            let block_dir = root.path().join(block);
            std::fs::create_dir_all(&block_dir).unwrap();
            std::fs::write(block_dir.join(target_file), b"target").unwrap();
            std::fs::write(block_dir.join(sibling_file), b"sibling").unwrap();
        }
        let namespace = ForkCacheNamespace {
            chain_cache_dir: root.path().to_path_buf(),
            file_name: target_file.to_string(),
        };

        namespace.invalidate().unwrap();

        for block in ["1", "2"] {
            let block_dir = root.path().join(block);
            assert!(!block_dir.join(target_file).exists());
            assert!(block_dir.join(sibling_file).exists());
        }
        assert!(regular_cache_entry.exists());
    }

    #[test]
    fn startup_fork_cache_user_rolls_back_after_database_flush() {
        let root = tempdir().unwrap();
        let block_cache_dir = root.path().join("1");
        let cache_path = block_cache_dir.join("storage.json");
        let cache_db = test_cache_db(cache_path.clone());
        let user = StagedForkDbUser {
            db: Some(Arc::new(tokio::sync::RwLock::new(CacheFlushingDb(cache_db.clone())))),
            cache_lease: StagedForkCacheLease::new(cache_db.clone(), Some(cache_path.clone())),
        };

        drop(user);

        assert!(cache_db.block_hashes().read().is_empty());
        assert!(!cache_path.exists());
    }

    #[test]
    fn startup_fork_cache_user_preserves_cache_after_commit() {
        let root = tempdir().unwrap();
        let block_cache_dir = root.path().join("1");
        let cache_path = block_cache_dir.join("storage.json");
        let cache_db = test_cache_db(cache_path.clone());
        let user = StagedForkDbUser {
            db: Some(Arc::new(tokio::sync::RwLock::new(CacheFlushingDb(cache_db.clone())))),
            cache_lease: StagedForkCacheLease::new(cache_db.clone(), Some(cache_path.clone())),
        };
        user.cache_lease.disarm();

        drop(user);

        assert_eq!(cache_db.block_hashes().read().get(&U256::ONE), Some(&B256::repeat_byte(0x22)));
        assert!(cache_path.exists());
    }

    #[test]
    fn fork_cache_source_invalidates_only_same_url_authoritative_replacements() {
        let anonymous = test_endpoint_identity(None, None);
        let anonymous_source = ForkCacheSource {
            rpc_url: "http://localhost".to_string(),
            endpoint_identity: anonymous,
        };
        let mut changed_anonymous = anonymous;
        changed_anonymous.source_chain_id = 2;

        assert!(
            !anonymous_source
                .authoritative_identity_changed_at_same_url("http://localhost", changed_anonymous)
        );
        assert!(
            !anonymous_source
                .authoritative_identity_changed_at_same_url("http://mirror", anonymous)
        );

        let hardfork = Some(FoundryHardfork::Ethereum(EthereumHardfork::Prague));
        let authoritative = test_endpoint_identity(hardfork, Some(B256::repeat_byte(0x22)));
        let authoritative_source = ForkCacheSource {
            rpc_url: "http://localhost".to_string(),
            endpoint_identity: authoritative,
        };
        let replacement = test_endpoint_identity(hardfork, Some(B256::repeat_byte(0x33)));

        assert!(
            !authoritative_source
                .authoritative_identity_changed_at_same_url("http://localhost", authoritative)
        );
        assert!(
            authoritative_source
                .authoritative_identity_changed_at_same_url("http://localhost", replacement)
        );
        assert!(
            anonymous_source
                .authoritative_identity_changed_at_same_url("http://localhost", authoritative)
        );
        assert!(
            authoritative_source
                .authoritative_identity_changed_at_same_url("http://localhost", anonymous)
        );
    }

    #[tokio::test]
    async fn test_deterministic_block_mining() {
        // Test that mine_block produces deterministic block hashes with same initial conditions
        let genesis_timestamp = 1743944919u64;

        // Create two identical backends
        let config_a = NodeConfig::test().with_genesis_timestamp(genesis_timestamp.into());
        let config_b = NodeConfig::test().with_genesis_timestamp(genesis_timestamp.into());

        let (api_a, _handle_a) = spawn(config_a).await;
        let (api_b, _handle_b) = spawn(config_b).await;

        // Mine empty blocks (no transactions) on both backends
        let outcome_a_1 = api_a.backend.mine_block(vec![]).await.unwrap();
        let outcome_b_1 = api_b.backend.mine_block(vec![]).await.unwrap();

        // Both should mine the same block number
        assert_eq!(outcome_a_1.block_number, outcome_b_1.block_number);

        // Get the actual blocks to compare hashes
        let block_a_1 =
            api_a.block_by_number(outcome_a_1.block_number.into()).await.unwrap().unwrap();
        let block_b_1 =
            api_b.block_by_number(outcome_b_1.block_number.into()).await.unwrap().unwrap();

        // The block hashes should be identical
        assert_eq!(
            block_a_1.header.hash, block_b_1.header.hash,
            "Block hashes should be deterministic. Got {} vs {}",
            block_a_1.header.hash, block_b_1.header.hash
        );

        // Mine another block to ensure it remains deterministic
        let outcome_a_2 = api_a.backend.mine_block(vec![]).await.unwrap();
        let outcome_b_2 = api_b.backend.mine_block(vec![]).await.unwrap();

        let block_a_2 =
            api_a.block_by_number(outcome_a_2.block_number.into()).await.unwrap().unwrap();
        let block_b_2 =
            api_b.block_by_number(outcome_b_2.block_number.into()).await.unwrap().unwrap();

        assert_eq!(
            block_a_2.header.hash, block_b_2.header.hash,
            "Second block hashes should also be deterministic. Got {} vs {}",
            block_a_2.header.hash, block_b_2.header.hash
        );

        // Ensure the blocks are different (sanity check)
        assert_ne!(
            block_a_1.header.hash, block_a_2.header.hash,
            "Different blocks should have different hashes"
        );
    }

    #[cfg(feature = "monad")]
    #[tokio::test]
    async fn monad_load_state_rebuilds_participant_cache() {
        use alloy_network::TransactionBuilder as _;
        use alloy_provider::Provider as _;

        let config = || {
            NodeConfig::test_monad()
                .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadNine.into()))
        };
        let (api, handle) = spawn(config()).await;
        let provider = handle.http_provider();
        let accounts = provider.get_accounts().await.unwrap();
        let sender = accounts[0];

        let receipt = provider
            .send_transaction(
                alloy_rpc_types::TransactionRequest::default()
                    .with_from(sender)
                    .with_to(accounts[1])
                    .with_value(U256::ONE)
                    .into(),
            )
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        let block_hash = receipt.block_hash.unwrap();
        let state = api.serialized_state(false).await.unwrap();

        let (loaded_api, _handle) = spawn(config()).await;
        loaded_api.backend.load_state(state).await.unwrap();

        let storage = loaded_api.backend.blockchain.storage.read();
        let participants = storage.monad_block_participants.get(&block_hash).unwrap();
        assert!(participants.contains(&sender));
    }

    #[cfg(feature = "monad")]
    #[tokio::test]
    async fn monad_load_state_restores_pruned_participant_cache() {
        use alloy_consensus::BlockHeader as _;
        use alloy_network::TransactionBuilder as _;
        use alloy_provider::Provider as _;

        let config = || {
            NodeConfig::test_monad()
                .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadNine.into()))
                .with_transaction_block_keeper(Some(1usize))
        };
        let (api, handle) = spawn(config()).await;
        let provider = handle.http_provider();
        let accounts = provider.get_accounts().await.unwrap();
        let sender = accounts[0];

        let first_receipt = provider
            .send_transaction(
                alloy_rpc_types::TransactionRequest::default()
                    .with_from(sender)
                    .with_to(accounts[1])
                    .with_value(U256::ONE)
                    .into(),
            )
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        let second_receipt = provider
            .send_transaction(
                alloy_rpc_types::TransactionRequest::default()
                    .with_from(sender)
                    .with_to(accounts[1])
                    .with_value(U256::ONE)
                    .into(),
            )
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        let first_block_hash = first_receipt.block_hash.unwrap();
        let second_block_hash = second_receipt.block_hash.unwrap();

        {
            let storage = api.backend.blockchain.storage.read();
            let first_block = storage.blocks.get(&first_block_hash).unwrap();
            assert!(first_block.body.transactions.is_empty());
            assert_ne!(
                first_block.header.transactions_root(),
                alloy_consensus::constants::EMPTY_ROOT_HASH
            );
            assert!(storage.monad_block_participants[&first_block_hash].contains(&sender));
            assert!(!storage.blocks[&second_block_hash].body.transactions.is_empty());
        }

        let state = api.serialized_state(false).await.unwrap();
        assert!(state.monad_block_participants[&first_block_hash].contains(&sender));
        assert_eq!(state.monad_block_participants.len(), 2);
        assert!(state.monad_block_participants[&second_block_hash].contains(&sender));

        let (loaded_api, _handle) = spawn(config()).await;
        loaded_api.backend.load_state(state).await.unwrap();

        {
            let storage = loaded_api.backend.blockchain.storage.read();
            assert!(storage.monad_block_participants[&first_block_hash].contains(&sender));
            assert!(storage.monad_block_participants[&second_block_hash].contains(&sender));
        }

        let outcome = loaded_api.backend.mine_block(vec![]).await.unwrap();
        assert_eq!(outcome.block_number, 3);
    }

    #[cfg(feature = "monad")]
    #[tokio::test]
    async fn monad_load_state_rejection_is_atomic() {
        use alloy_consensus::BlockHeader as _;
        use alloy_network::TransactionBuilder as _;
        use alloy_provider::Provider as _;

        let config = || {
            NodeConfig::test_monad()
                .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadNine.into()))
                .with_transaction_block_keeper(Some(1usize))
        };
        let (source_api, source_handle) = spawn(config()).await;
        let source_provider = source_handle.http_provider();
        let accounts = source_provider.get_accounts().await.unwrap();
        let sentinel = alloy_primitives::Address::repeat_byte(0x77);

        source_api.anvil_set_balance(sentinel, U256::from(123)).await.unwrap();
        for nonce in 0..2 {
            source_provider
                .send_transaction(
                    alloy_rpc_types::TransactionRequest::default()
                        .with_from(accounts[0])
                        .with_to(accounts[1])
                        .with_nonce(nonce)
                        .with_value(U256::ONE)
                        .into(),
                )
                .await
                .unwrap()
                .get_receipt()
                .await
                .unwrap();
        }

        let mut invalid_state = source_api.serialized_state(false).await.unwrap();
        let pruned_block_hash = invalid_state
            .blocks
            .iter()
            .find(|block| {
                block.transactions.is_empty()
                    && block.header.transactions_root()
                        != alloy_consensus::constants::EMPTY_ROOT_HASH
            })
            .unwrap()
            .header
            .hash_slow();
        assert!(invalid_state.monad_block_participants.remove(&pruned_block_hash).is_some());

        let (target_api, _handle) = spawn(config()).await;
        target_api.anvil_set_balance(sentinel, U256::from(7)).await.unwrap();
        target_api.mine_one().await.unwrap();
        let original_best_hash = target_api.backend.best_hash();
        let original_best_number = target_api.backend.best_number();
        let original_block_env = target_api.backend.evm_env.read().block_env.clone();
        let original_balance = target_api.backend.current_balance(sentinel).await.unwrap();

        let err = target_api.backend.load_state(invalid_state).await.unwrap_err();
        assert!(matches!(err, super::BlockchainError::DataUnavailable));
        assert_eq!(target_api.backend.best_hash(), original_best_hash);
        assert_eq!(target_api.backend.best_number(), original_best_number);
        assert_eq!(target_api.backend.evm_env.read().block_env, original_block_env);
        assert_eq!(target_api.backend.current_balance(sentinel).await.unwrap(), original_balance);

        let outcome = target_api.backend.mine_block(vec![]).await.unwrap();
        assert_eq!(outcome.block_number, original_best_number + 1);
    }

    #[tokio::test]
    async fn trace_decoder_follows_executed_hardfork_for_cross_namespace_override() {
        let (api, _) = spawn(
            NodeConfig::test_tempo()
                .with_hardfork(Some(FoundryHardfork::Ethereum(EthereumHardfork::Prague))),
        )
        .await;

        let decoder = api.backend.call_trace_decoder();
        assert_eq!(decoder.hardfork(), Some(FoundryHardfork::Tempo(api.backend.tempo_hardfork())));
        // The refresh compares the same coerced value, so repeated calls stay stable.
        assert!(Arc::ptr_eq(&decoder, &api.backend.call_trace_decoder()));
    }

    #[cfg(feature = "monad")]
    #[tokio::test]
    async fn monad_trace_decoder_follows_resolved_hardfork() {
        let (monad_eight, _) = spawn(
            NodeConfig::test_monad()
                .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadEight.into())),
        )
        .await;
        let stale_monad_nine = foundry_evm_traces::CallTraceDecoderBuilder::new()
            .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadNine.into()))
            .build();
        *monad_eight.backend.call_trace_decoder.write() = Arc::new(stale_monad_nine);

        let decoder = monad_eight.backend.call_trace_decoder();
        assert_eq!(
            decoder.hardfork(),
            Some(foundry_evm_hardforks::MonadHardfork::MonadEight.into())
        );
        assert!(
            !decoder.labels.contains_key(&monad_evm2::reserve_balance::RESERVE_BALANCE_ADDRESS)
        );

        let (monad_nine, _) = spawn(
            NodeConfig::test_monad()
                .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadNine.into())),
        )
        .await;
        let stale_monad_eight = foundry_evm_traces::CallTraceDecoderBuilder::new()
            .with_hardfork(Some(foundry_evm_hardforks::MonadHardfork::MonadEight.into()))
            .build();
        *monad_nine.backend.call_trace_decoder.write() = Arc::new(stale_monad_eight);

        let decoder = monad_nine.backend.call_trace_decoder();
        assert_eq!(
            decoder.hardfork(),
            Some(foundry_evm_hardforks::MonadHardfork::MonadNine.into())
        );
        assert_eq!(
            decoder
                .labels
                .get(&monad_evm2::reserve_balance::RESERVE_BALANCE_ADDRESS)
                .map(String::as_str),
            Some("ReserveBalance")
        );
    }

    #[cfg(feature = "base")]
    #[tokio::test]
    async fn base_trace_decoder_follows_resolved_upgrade() {
        let (azul, _) =
            spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Azul.into()))).await;
        let stale_beryl =
            CallTraceDecoderBuilder::new().with_hardfork(Some(BaseUpgrade::Beryl.into())).build();
        *azul.backend.call_trace_decoder.write() = Arc::new(stale_beryl);

        let decoder = azul.backend.call_trace_decoder();
        assert_eq!(decoder.hardfork(), Some(BaseUpgrade::Azul.into()));
        assert!(!decoder.labels.values().any(|label| label == "B20Factory"));
        assert!(!decoder.labels.values().any(|label| label == "ActivationRegistry"));

        let (beryl, _) =
            spawn(NodeConfig::test_base().with_hardfork(Some(BaseUpgrade::Beryl.into()))).await;
        let stale_azul =
            CallTraceDecoderBuilder::new().with_hardfork(Some(BaseUpgrade::Azul.into())).build();
        *beryl.backend.call_trace_decoder.write() = Arc::new(stale_azul);

        let decoder = beryl.backend.call_trace_decoder();
        assert_eq!(decoder.hardfork(), Some(BaseUpgrade::Beryl.into()));
        assert!(decoder.labels.values().any(|label| label == "B20Factory"));
        assert!(decoder.labels.values().any(|label| label == "ActivationRegistry"));
    }
}
