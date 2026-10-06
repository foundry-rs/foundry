use crate::impersonation::ImpersonationState;
use alloy_consensus::{Transaction, transaction::TxHashRef};
use alloy_primitives::U256;
use eyre::Result;
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
        CoinbaseTipOrdering, EthPooledTransaction, EthTransactionValidator, Pool, PoolTransaction,
        TransactionOrigin, TransactionValidationOutcome, TransactionValidationTaskExecutor,
        TransactionValidator, blobstore::DiskFileBlobStore, validate::ValidTransaction,
    },
    primitives::{BlockBody, SealedBlock},
};
use std::fmt::{self, Debug};

/// Wraps the standard Ethereum validator and short-circuits validation for impersonated
/// accounts.
pub struct AnvilValidator<V> {
    inner: V,
    state: ImpersonationState,
}

impl<V: Debug> Debug for AnvilValidator<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnvilValidator").field("inner", &self.inner).finish_non_exhaustive()
    }
}

impl<V> TransactionValidator for AnvilValidator<V>
where
    V: TransactionValidator,
    V::Transaction: PoolTransaction,
{
    type Transaction = V::Transaction;
    type Block = V::Block;

    async fn validate_transaction(
        &self,
        origin: TransactionOrigin,
        transaction: Self::Transaction,
    ) -> TransactionValidationOutcome<Self::Transaction> {
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

        self.inner.validate_transaction(origin, transaction).await
    }

    fn on_new_head_block(&self, new_tip_block: &SealedBlock<Self::Block>) {
        self.state
            .forget_tx_senders(new_tip_block.body().transactions().iter().map(|tx| *tx.tx_hash()));
        self.inner.on_new_head_block(new_tip_block);
    }
}

/// Pool builder that wraps the default Ethereum pool builder and decorates the validator with
/// impersonation support.
#[derive(Debug, Clone)]
pub struct AnvilPoolBuilder {
    /// The shared impersonation state.
    pub state: ImpersonationState,
}

/// The transaction pool type produced by [`AnvilPoolBuilder`].
pub type AnvilTransactionPool<Provider, Evm> = Pool<
    TransactionValidationTaskExecutor<
        AnvilValidator<EthTransactionValidator<Provider, EthPooledTransaction, Evm>>,
    >,
    CoinbaseTipOrdering<EthPooledTransaction>,
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

        let validator =
            TransactionValidationTaskExecutor::eth_builder(ctx.provider().clone(), evm_config)
                .kzg_settings(ctx.kzg_settings()?)
                .with_max_tx_input_bytes(ctx.config().txpool.max_tx_input_bytes)
                .with_local_transactions_config(pool_config.local_transactions_config.clone())
                .set_tx_fee_cap(ctx.config().rpc.rpc_tx_fee_cap)
                .with_max_tx_gas_limit(ctx.config().txpool.max_tx_gas_limit)
                .with_minimum_priority_fee(ctx.config().txpool.minimum_priority_fee)
                .with_additional_tasks(ctx.config().txpool.additional_validation_tasks)
                .build_with_tasks(ctx.task_executor().clone(), blob_store.clone())
                .map(|inner| AnvilValidator { inner, state: self.state.clone() });

        TxPoolBuilder::new(ctx)
            .with_validator(validator)
            .build_and_spawn_maintenance_task(blob_store, pool_config)
    }
}
