use crate::{config::NodeConfig, impersonation::ImpersonationState, types::TransactionOrder};
use alloy_consensus::{Transaction, transaction::TxHashRef};
use alloy_primitives::{B256, Signature, U256};
use eyre::Result;
use parking_lot::Mutex;
use reth_ethereum::{
    TransactionSigned,
    chainspec::EthereumHardforks,
    node::{
        api::{ConfigureEvm, NodePrimitives, PrimitivesTy},
        builder::{
            BuilderContext, FullNodeTypes, NodeTypes,
            components::{PoolBuilder, TxPoolBuilder, create_blob_store_with_cache},
        },
    },
    pool::{
        EthPooledTransaction, EthTransactionValidator, Pool, PoolTransaction, Priority,
        TransactionOrdering, TransactionOrigin, TransactionValidationOutcome,
        TransactionValidationTaskExecutor, TransactionValidator,
        blobstore::DiskFileBlobStore,
        error::{InvalidPoolTransactionError, PoolTransactionError},
        validate::ValidTransaction,
    },
    primitives::{BlockBody, Recovered, SealedBlock},
};
use std::{
    any::Any,
    collections::HashMap,
    fmt::{self, Debug, Display},
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Transactions whose ECDSA signature `anvil_impersonateSignature` can override.
pub trait TxSignature {
    /// Returns the signature, if the transaction carries an ECDSA signature.
    fn ecdsa_signature(&self) -> Option<&Signature>;
}

impl TxSignature for TransactionSigned {
    fn ecdsa_signature(&self) -> Option<&Signature> {
        Some(self.signature())
    }
}

/// Wraps the standard Ethereum validator and short-circuits validation for impersonated
/// accounts.
pub struct AnvilValidator<V> {
    inner: V,
    state: ImpersonationState,
    disable_balance_checks: bool,
}

impl<V: Debug> Debug for AnvilValidator<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilValidator").field("inner", &self.inner).finish_non_exhaustive()
    }
}

impl<V> TransactionValidator for AnvilValidator<V>
where
    V: TransactionValidator,
    V::Transaction: PoolTransaction<Consensus: TxSignature>,
{
    type Transaction = V::Transaction;
    type Block = V::Block;

    async fn validate_transaction(
        &self,
        origin: TransactionOrigin,
        transaction: Self::Transaction,
    ) -> TransactionValidationOutcome<Self::Transaction> {
        if self.state.is_dropped(transaction.hash()) {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidPoolTransactionError::Other(Box::new(RevertedTransaction)),
            );
        }
        // A signature override attributes the transaction to the chosen sender. The pool keeps
        // the recovered sender for ordering; execution and lookups use the override.
        let signature_sender = if self.state.has_signature_overrides() {
            transaction
                .clone_into_consensus()
                .ecdsa_signature()
                .and_then(|signature| self.state.signature_override(signature))
        } else {
            None
        };
        if let Some(sender) = signature_sender {
            self.state.remember_tx_sender(*transaction.hash(), sender);
            // Rebuild the pool transaction with the chosen sender, so the block builder executes
            // it from that account.
            let (tx, _) = transaction.clone_into_consensus().into_parts();
            let transaction =
                match Self::Transaction::try_from_consensus(Recovered::new_unchecked(tx, sender)) {
                    Ok(rebuilt) => rebuilt,
                    Err(_) => transaction,
                };
            return TransactionValidationOutcome::Valid {
                balance: U256::MAX,
                state_nonce: transaction.nonce(),
                bytecode_hash: None,
                transaction: ValidTransaction::Valid(transaction),
                propagate: true,
                authorities: None,
            };
        }
        if self.state.is_impersonated(&transaction.sender()) {
            self.state.remember_tx_sender(*transaction.hash(), transaction.sender());
            return TransactionValidationOutcome::Valid {
                balance: U256::MAX,
                state_nonce: transaction.nonce(),
                bytecode_hash: None,
                transaction: ValidTransaction::Valid(transaction),
                propagate: true,
                authorities: None,
            };
        }

        let mut outcome = self.inner.validate_transaction(origin, transaction).await;
        if self.disable_balance_checks
            && let TransactionValidationOutcome::Valid { balance, .. } = &mut outcome
        {
            // The pool parks transactions the sender cannot afford. Report an unlimited balance,
            // so a transaction funded earlier in the same block is mined.
            *balance = U256::MAX;
        }
        outcome
    }

    fn on_new_head_block(&self, new_tip_block: &SealedBlock<Self::Block>) {
        self.state
            .forget_tx_senders(new_tip_block.body().transactions().iter().map(|tx| *tx.tx_hash()));
        self.inner.on_new_head_block(new_tip_block);
    }
}

/// The pool error for a transaction that a revert removed from the chain.
#[derive(Debug)]
struct RevertedTransaction;

impl Display for RevertedTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("transaction was removed from the chain by a revert")
    }
}

impl std::error::Error for RevertedTransaction {}

impl PoolTransactionError for RevertedTransaction {
    fn is_bad_transaction(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Orders transactions by effective tip, or in order of arrival for `--order fifo`.
///
/// The arrival order is the order in which the pool first asked for a transaction's priority. The
/// bookkeeping is cleared when it grows large, which only reorders transactions still in the pool
/// among themselves.
pub struct AnvilOrdering<T> {
    order: TransactionOrder,
    next: Arc<AtomicU64>,
    arrivals: Arc<Mutex<HashMap<B256, u64>>>,
    _tx: PhantomData<T>,
}

impl<T> Debug for AnvilOrdering<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilOrdering").field("order", &self.order).finish_non_exhaustive()
    }
}

impl<T> Clone for AnvilOrdering<T> {
    fn clone(&self) -> Self {
        Self {
            order: self.order,
            next: self.next.clone(),
            arrivals: self.arrivals.clone(),
            _tx: PhantomData,
        }
    }
}

impl<T> AnvilOrdering<T> {
    const MAX_TRACKED_ARRIVALS: usize = 100_000;

    /// Creates the ordering.
    pub fn new(order: TransactionOrder) -> Self {
        Self {
            order,
            next: Arc::new(AtomicU64::new(0)),
            arrivals: Arc::new(Mutex::new(HashMap::new())),
            _tx: PhantomData,
        }
    }
}

impl<T> TransactionOrdering for AnvilOrdering<T>
where
    T: PoolTransaction + 'static,
{
    type PriorityValue = u128;
    type Transaction = T;

    fn priority(&self, transaction: &Self::Transaction, base_fee: u64) -> Priority<u128> {
        match self.order {
            TransactionOrder::Fees => transaction.effective_tip_per_gas(base_fee).into(),
            TransactionOrder::Fifo => {
                let mut arrivals = self.arrivals.lock();
                if arrivals.len() > Self::MAX_TRACKED_ARRIVALS {
                    arrivals.clear();
                }
                let arrival = *arrivals
                    .entry(*transaction.hash())
                    .or_insert_with(|| self.next.fetch_add(1, Ordering::Relaxed));
                Priority::Value(u128::from(u64::MAX - arrival))
            }
        }
    }
}

/// The pool validation knobs anvil exposes.
#[derive(Clone, Copy, Debug)]
pub struct PoolSettings {
    /// The block gas limit the validator enforces until the first block is mined.
    pub block_gas_limit: u64,
    /// Whether the validator skips the sender balance check.
    pub disable_balance_checks: bool,
    /// Whether the pool enforces no minimum priority fee.
    pub disable_min_priority_fee: bool,
}

impl PoolSettings {
    /// Reads the settings from the node config.
    pub fn from_config(config: &NodeConfig) -> Self {
        Self {
            block_gas_limit: config.get_gas_limit(),
            disable_balance_checks: config.disable_pool_balance_checks,
            disable_min_priority_fee: config.disable_min_priority_fee,
        }
    }
}

/// Pool builder that wraps the default Ethereum pool builder and decorates the validator with
/// impersonation support.
#[derive(Debug, Clone)]
pub struct AnvilPoolBuilder {
    /// The shared impersonation state.
    pub state: ImpersonationState,
    /// How the pool orders transactions.
    pub order: TransactionOrder,
    /// The validation knobs.
    pub settings: PoolSettings,
}

/// The transaction pool type produced by [`AnvilPoolBuilder`].
pub type AnvilTransactionPool<Provider, Evm> = Pool<
    TransactionValidationTaskExecutor<
        AnvilValidator<EthTransactionValidator<Provider, EthPooledTransaction, Evm>>,
    >,
    AnvilOrdering<EthPooledTransaction>,
    DiskFileBlobStore,
>;

impl<Types, Node, Evm> PoolBuilder<Node, Evm> for AnvilPoolBuilder
where
    Types: NodeTypes<
            ChainSpec: EthereumHardforks,
            Primitives: NodePrimitives<SignedTx = TransactionSigned>,
        >,
    Node: FullNodeTypes<Types = Types>,
    Evm: ConfigureEvm<Primitives = PrimitivesTy<Types>> + Clone + 'static,
{
    type Pool = AnvilTransactionPool<Node::Provider, Evm>;

    async fn build_pool(self, ctx: &BuilderContext<Node>, evm_config: Evm) -> Result<Self::Pool> {
        let pool_config = ctx.pool_config();
        let blob_store = create_blob_store_with_cache(ctx, None)?;

        let minimum_priority_fee = if self.settings.disable_min_priority_fee {
            None
        } else {
            ctx.config().txpool.minimum_priority_fee
        };
        let mut validator =
            TransactionValidationTaskExecutor::eth_builder(ctx.provider().clone(), evm_config)
                .kzg_settings(ctx.kzg_settings()?)
                .with_max_tx_input_bytes(ctx.config().txpool.max_tx_input_bytes)
                .with_local_transactions_config(pool_config.local_transactions_config.clone())
                .set_tx_fee_cap(ctx.config().rpc.rpc_tx_fee_cap)
                .with_max_tx_gas_limit(ctx.config().txpool.max_tx_gas_limit)
                .with_minimum_priority_fee(minimum_priority_fee)
                .with_additional_tasks(ctx.config().txpool.additional_validation_tasks)
                .set_block_gas_limit(self.settings.block_gas_limit);
        if self.settings.disable_balance_checks {
            validator = validator.disable_balance_check();
        }
        let validator = validator
            .build_with_tasks(ctx.task_executor().clone(), blob_store.clone())
            .map(|inner| AnvilValidator {
                inner,
                state: self.state.clone(),
                disable_balance_checks: self.settings.disable_balance_checks,
            });

        TxPoolBuilder::new(ctx)
            .with_validator(validator)
            .build_with_ordering_and_spawn_maintenance_task(
                AnvilOrdering::new(self.order),
                blob_store,
                pool_config,
            )
    }
}
