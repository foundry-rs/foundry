use crate::{config::NodeConfig, impersonation::ImpersonationState, types::TransactionOrder};
use alloy_consensus::{
    BlockHeader, Transaction, Typed2718, constants::EIP4844_TX_TYPE_ID, transaction::TxHashRef,
};
use alloy_primitives::{B256, Signature, U256};
use eyre::Result;
use parking_lot::{Mutex, RwLock};
use reth_ethereum::{
    TransactionSigned,
    chainspec::{EthereumHardfork, EthereumHardforks},
    node::{
        api::{ConfigureEvm, NodePrimitives, PrimitivesTy},
        builder::{
            BuilderContext, FullNodeTypes, NodeTypes,
            components::{PoolBuilder, TxPoolBuilder, create_blob_store_with_cache},
        },
    },
    pool::{
        EthPooledTransaction, EthTransactionValidator, Pool, PoolTransaction, PriceBumpConfig,
        Priority, TransactionOrdering, TransactionOrigin, TransactionValidationOutcome,
        TransactionValidationTaskExecutor, TransactionValidator,
        blobstore::DiskFileBlobStore,
        error::{InvalidPoolTransactionError, PoolTransactionError},
        validate::ValidTransaction,
    },
    primitives::{
        BlockBody, GotExpected, Recovered, SealedBlock, transaction::error::InvalidTransactionError,
    },
    storage::BlockReaderIdExt,
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
    settings: PoolSettings,
    /// The base fee of the latest block, for the gas-only balance rule.
    base_fee: Arc<AtomicU64>,
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
        // Reth puts the transactions of reverted blocks back into the pool as external ones;
        // anvil leaves them out. A user who sends one again gets it mined again.
        if self.state.is_dropped(transaction.hash()) {
            if origin.is_external() {
                return TransactionValidationOutcome::Invalid(
                    transaction,
                    InvalidPoolTransactionError::Other(Box::new(RevertedTransaction)),
                );
            }
            self.state.undrop_tx(transaction.hash());
        }
        if self.settings.reject_blob_transactions && transaction.ty() == EIP4844_TX_TYPE_ID {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidPoolTransactionError::Other(Box::new(BlobTransactionsUnsupported)),
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

        let outcome = self.inner.validate_transaction(origin, transaction).await;
        // Anvil mines a transaction from a sender with code: EIP-3607 is off for mining, as it is
        // for calls. Reth's validator rejects it, so accept it here as a ready transaction. On
        // Arbitrum, anvil also mines a transaction whose priority fee is above its fee cap.
        if let TransactionValidationOutcome::Invalid(
            transaction,
            InvalidPoolTransactionError::Consensus(error),
        ) = &outcome
            && (matches!(error, InvalidTransactionError::SignerAccountHasBytecode)
                || (self.settings.allow_tip_above_fee_cap
                    && matches!(error, InvalidTransactionError::TipAboveFeeCap)))
        {
            let transaction = transaction.clone();
            return TransactionValidationOutcome::Valid {
                balance: U256::MAX,
                state_nonce: transaction.nonce(),
                bytecode_hash: None,
                transaction: ValidTransaction::Valid(transaction),
                propagate: true,
                authorities: None,
            };
        }
        let TransactionValidationOutcome::Valid {
            balance,
            state_nonce,
            bytecode_hash,
            transaction,
            propagate,
            authorities,
        } = outcome
        else {
            return outcome;
        };
        let balance = match self.settings.balance_rule {
            BalanceRule::Full => balance,
            // The pool parks transactions the sender cannot afford. Report an unlimited balance,
            // so a transaction funded earlier in the same block is mined.
            BalanceRule::None => U256::MAX,
            BalanceRule::GasOnly => {
                let tx = transaction.transaction();
                let base_fee = self.base_fee.load(Ordering::Relaxed);
                let price = tx.clone_into_consensus().effective_gas_price(Some(base_fee));
                let required = U256::from(tx.gas_limit()).saturating_mul(U256::from(price));
                if balance < required {
                    return TransactionValidationOutcome::Invalid(
                        transaction.into_transaction(),
                        InvalidTransactionError::InsufficientFunds(
                            GotExpected { got: balance, expected: required }.into(),
                        )
                        .into(),
                    );
                }
                U256::MAX
            }
        };
        TransactionValidationOutcome::Valid {
            balance,
            state_nonce,
            bytecode_hash,
            transaction,
            propagate,
            authorities,
        }
    }

    fn on_new_head_block(&self, new_tip_block: &SealedBlock<Self::Block>) {
        self.state
            .forget_tx_senders(new_tip_block.body().transactions().iter().map(|tx| *tx.tx_hash()));
        self.base_fee.store(
            new_tip_block.header().base_fee_per_gas().unwrap_or_default(),
            Ordering::Relaxed,
        );
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

/// The pool error for a blob transaction on a network without blobs.
#[derive(Debug)]
struct BlobTransactionsUnsupported;

impl Display for BlobTransactionsUnsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EIP-4844 blob transactions are not supported on Monad")
    }
}

impl std::error::Error for BlobTransactionsUnsupported {}

impl PoolTransactionError for BlobTransactionsUnsupported {
    fn is_bad_transaction(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The transaction order of the pool. It can change at runtime; the change applies to the
/// transactions added from then on.
#[derive(Clone, Debug)]
pub struct SharedTransactionOrder(Arc<RwLock<TransactionOrder>>);

impl SharedTransactionOrder {
    /// Creates the shared order.
    pub fn new(order: TransactionOrder) -> Self {
        Self(Arc::new(RwLock::new(order)))
    }

    /// Returns the current order.
    pub fn get(&self) -> TransactionOrder {
        *self.0.read()
    }

    /// Sets the order.
    pub fn set(&self, order: TransactionOrder) {
        *self.0.write() = order;
    }
}

/// Orders transactions by effective tip, or in order of arrival for `--order fifo`.
///
/// The arrival order is the order in which the pool first asked for a transaction's priority. The
/// bookkeeping is cleared when it grows large, which only reorders transactions still in the pool
/// among themselves.
pub struct AnvilOrdering<T> {
    order: SharedTransactionOrder,
    next: Arc<AtomicU64>,
    arrivals: Arc<Mutex<HashMap<B256, u64>>>,
    _tx: PhantomData<T>,
}

impl<T> Debug for AnvilOrdering<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilOrdering").field("order", &self.order.get()).finish_non_exhaustive()
    }
}

impl<T> Clone for AnvilOrdering<T> {
    fn clone(&self) -> Self {
        Self {
            order: self.order.clone(),
            next: self.next.clone(),
            arrivals: self.arrivals.clone(),
            _tx: PhantomData,
        }
    }
}

impl<T> AnvilOrdering<T> {
    const MAX_TRACKED_ARRIVALS: usize = 100_000;

    /// Creates the ordering.
    pub fn new(order: SharedTransactionOrder) -> Self {
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
        match self.order.get() {
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

/// How the pool checks that a sender can pay for a transaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BalanceRule {
    /// The value plus the gas limit at the maximum fee, as Ethereum does.
    #[default]
    Full,
    /// Only the gas limit at the effective gas price, as Monad does. The value is charged, or the
    /// transaction fails, when it executes.
    #[cfg_attr(not(feature = "monad"), expect(dead_code))]
    GasOnly,
    /// No check.
    None,
}

/// The pool validation knobs anvil exposes.
#[derive(Clone, Copy, Debug)]
pub struct PoolSettings {
    /// The block gas limit the validator enforces until the first block is mined.
    pub block_gas_limit: u64,
    /// How the validator checks the sender balance.
    pub balance_rule: BalanceRule,
    /// Whether the pool enforces no minimum priority fee.
    pub disable_min_priority_fee: bool,
    /// Whether the pool rejects EIP-4844 blob transactions.
    pub reject_blob_transactions: bool,
    /// Whether a priority fee above the fee cap is allowed, as on Arbitrum.
    pub allow_tip_above_fee_cap: bool,
}

impl PoolSettings {
    /// Reads the settings from the node config.
    pub fn from_config(config: &NodeConfig) -> Self {
        Self {
            block_gas_limit: config.get_gas_limit(),
            balance_rule: if config.disable_pool_balance_checks {
                BalanceRule::None
            } else {
                BalanceRule::Full
            },
            disable_min_priority_fee: config.disable_min_priority_fee,
            reject_blob_transactions: false,
            allow_tip_above_fee_cap: config.is_arbitrum(),
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
    pub order: SharedTransactionOrder,
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
        let mut pool_config = ctx.pool_config();
        // Anvil has no minimum fee: a zero gas price is fine with a zero base fee.
        pool_config.minimal_protocol_basefee = 0;
        // Anvil replaces a pooled transaction with any higher fee.
        pool_config.price_bumps =
            PriceBumpConfig { default_price_bump: 0, replace_blob_tx_price_bump: 0 };
        let blob_store = create_blob_store_with_cache(ctx, None)?;

        let minimum_priority_fee = if self.settings.disable_min_priority_fee {
            None
        } else {
            ctx.config().txpool.minimum_priority_fee
        };
        // Anvil's hardfork is fixed, so the transaction types the latest block does not support
        // are rejected, as anvil does.
        let latest = ctx.provider().latest_header()?;
        let (number, timestamp) =
            latest.as_ref().map(|header| (header.number(), header.timestamp())).unwrap_or_default();
        let chain_spec = ctx.chain_spec();
        let active_at_block =
            |fork| chain_spec.ethereum_fork_activation(fork).active_at_block(number);
        let mut validator =
            TransactionValidationTaskExecutor::eth_builder(ctx.provider().clone(), evm_config)
                .set_eip2718(active_at_block(EthereumHardfork::Berlin))
                .set_eip1559(active_at_block(EthereumHardfork::London))
                .set_eip4844(chain_spec.is_cancun_active_at_timestamp(timestamp))
                .set_eip7702(chain_spec.is_prague_active_at_timestamp(timestamp))
                .kzg_settings(ctx.kzg_settings()?)
                .with_max_tx_input_bytes(ctx.config().txpool.max_tx_input_bytes)
                .with_local_transactions_config(pool_config.local_transactions_config.clone())
                .set_tx_fee_cap(ctx.config().rpc.rpc_tx_fee_cap)
                .with_max_tx_gas_limit(ctx.config().txpool.max_tx_gas_limit)
                .with_minimum_priority_fee(minimum_priority_fee)
                .with_additional_tasks(ctx.config().txpool.additional_validation_tasks)
                .set_block_gas_limit(self.settings.block_gas_limit);
        if self.settings.balance_rule != BalanceRule::Full {
            validator = validator.disable_balance_check();
        }
        let base_fee = latest.and_then(|header| header.base_fee_per_gas());
        let validator = validator
            .build_with_tasks(ctx.task_executor().clone(), blob_store.clone())
            .map(|inner| AnvilValidator {
                inner,
                state: self.state.clone(),
                settings: self.settings,
                base_fee: Arc::new(AtomicU64::new(base_fee.unwrap_or_default())),
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
