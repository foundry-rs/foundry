//! Tempo's `eth` API, as `tempo_node::rpc::TempoEthApi` implements it, with two changes for a
//! dev chain: the node builds a pending block, which Tempo turns off because a consensus node
//! cannot build one without its system transaction, and every simulated Tempo transaction gets
//! its own hash, so expiring nonces of two calls in one bundle do not collide. The native balance
//! is the account's, not Tempo's placeholder.

use alloy_eips::eip2718::WithEncoded;
use alloy_primitives::{B256, U256, keccak256};
use futures::TryFutureExt;
use reth_ethereum::{
    evm::primitives::{EvmEnvFor, TxEnvFor},
    pool::{PoolTx, TransactionOrigin},
    provider::ProviderError,
    rpc::{
        DynRpcConverter,
        eth::{
            EthApi, EthApiError, EthApiSettings, EthStateCache, FeeHistoryCache, GasPriceOracle,
            PendingBlock, RpcInvalidTransactionError, SignError,
        },
    },
    tasks::{
        Runtime,
        pool::{BlockingTaskGuard, BlockingTaskPool},
    },
};
use reth_rpc_eth_api::{
    EthApiTypes, FromEthApiError, IntoEthApiError, RpcNodeCore, RpcNodeCoreExt, RpcTxReq,
    helpers::{
        Call, EthApiSpec, EthBlocks, EthCall, EthFees, EthState, EthSubscriptions, EthTransactions,
        LoadBlock, LoadFee, LoadPendingBlock, LoadReceipt, LoadState, LoadTransaction,
        SpawnBlocking, Trace, bal::GetBlockAccessList, estimate::EstimateCall,
        pending_block::PendingEnvBuilder, spec::SignersForRpc,
    },
};
use reth_rpc_eth_types::builder::config::PendingBlockKind;
use revm::{Database, context::result::EVMError};
use std::sync::Arc;
use tempo_alloy::{TempoNetwork, rpc::TempoTransactionRequest};
use tempo_evm::{FeeTokenResolver, TempoStateAccess};
use tempo_node::rpc::{TempoEthApiBounds, error::TempoEthApiError};
use tempo_precompiles::{NONCE_PRECOMPILE_ADDRESS, nonce::NonceManager, storage::StorageActions};
use tempo_primitives::{TEMPO_GAS_PRICE_SCALING_FACTOR, transaction::TEMPO_EXPIRING_NONCE_KEY};
use tempo_transaction_pool::TempoTransactionPoolExt;
use tokio::sync::Mutex;

/// Tempo's `eth` API for a dev chain; see the module documentation.
#[derive(Debug, Clone)]
pub struct AnvilTempoEthApi<N: TempoEthApiBounds> {
    inner: EthApi<N, DynRpcConverter<N::Evm, TempoNetwork>>,
}

impl<N: TempoEthApiBounds> AnvilTempoEthApi<N> {
    /// Wraps reth's `eth` API with Tempo's converter.
    pub const fn new(inner: EthApi<N, DynRpcConverter<N::Evm, TempoNetwork>>) -> Self {
        Self { inner }
    }
}

impl<N: TempoEthApiBounds> EthApiTypes for AnvilTempoEthApi<N> {
    type Error = TempoEthApiError;
    type NetworkTypes = TempoNetwork;
    type RpcConvert = DynRpcConverter<N::Evm, TempoNetwork>;

    fn eth_api_settings(&self) -> &EthApiSettings {
        self.inner.eth_api_settings()
    }

    fn converter(&self) -> &Self::RpcConvert {
        self.inner.converter()
    }
}

impl<N: TempoEthApiBounds> RpcNodeCore for AnvilTempoEthApi<N> {
    type Primitives = N::Primitives;
    type Provider = N::Provider;
    type Pool = N::Pool;
    type Evm = N::Evm;
    type Network = N::Network;

    fn pool(&self) -> &Self::Pool {
        self.inner.pool()
    }

    fn evm_config(&self) -> &Self::Evm {
        self.inner.evm_config()
    }

    fn network(&self) -> &Self::Network {
        self.inner.network()
    }

    fn provider(&self) -> &Self::Provider {
        self.inner.provider()
    }
}

impl<N: TempoEthApiBounds> RpcNodeCoreExt for AnvilTempoEthApi<N> {
    fn cache(&self) -> &EthStateCache<N::Primitives> {
        self.inner.cache()
    }
}

impl<N: TempoEthApiBounds> EthApiSpec for AnvilTempoEthApi<N> {
    fn starting_block(&self) -> U256 {
        self.inner.starting_block()
    }
}

impl<N: TempoEthApiBounds> SpawnBlocking for AnvilTempoEthApi<N> {
    fn io_task_spawner(&self) -> &Runtime {
        self.inner.task_spawner()
    }

    fn tracing_task_pool(&self) -> &BlockingTaskPool {
        self.inner.blocking_task_pool()
    }

    fn tracing_task_guard(&self) -> &BlockingTaskGuard {
        self.inner.blocking_task_guard()
    }

    fn blocking_io_task_guard(&self) -> &Arc<tokio::sync::Semaphore> {
        self.inner.blocking_io_task_guard()
    }
}

impl<N: TempoEthApiBounds> LoadPendingBlock for AnvilTempoEthApi<N> {
    fn pending_block(&self) -> &Mutex<Option<PendingBlock<Self::Primitives>>> {
        self.inner.pending_block()
    }

    fn pending_env_builder(&self) -> &dyn PendingEnvBuilder<Self::Evm> {
        self.inner.pending_env_builder()
    }

    // A dev chain needs no consensus data, so it builds a pending block as reth does.
    fn pending_block_kind(&self) -> PendingBlockKind {
        self.inner.pending_block_kind()
    }
}

impl<N: TempoEthApiBounds> LoadFee for AnvilTempoEthApi<N> {
    fn gas_oracle(&self) -> &GasPriceOracle<Self::Provider> {
        self.inner.gas_oracle()
    }

    fn fee_history_cache(
        &self,
    ) -> &FeeHistoryCache<reth_ethereum::primitives::HeaderTy<N::Primitives>> {
        self.inner.fee_history_cache()
    }
}

impl<N: TempoEthApiBounds> LoadState for AnvilTempoEthApi<N> {
    async fn next_available_nonce_for(
        &self,
        request: &RpcTxReq<Self::NetworkTypes>,
    ) -> Result<u64, Self::Error> {
        let Some(nonce_key) = request.nonce_key.filter(|key| !key.is_zero()) else {
            return Ok(self.inner.next_available_nonce_for(request).await?);
        };
        if nonce_key == TEMPO_EXPIRING_NONCE_KEY {
            return Ok(0);
        }
        let Some(from) = request.from else {
            return Err(SignError::NoAccount.into_eth_err());
        };
        let slot = NonceManager::new().nonces[from][nonce_key].slot();
        self.spawn_blocking_io(move |this| {
            let on_chain_nonce: u64 = this
                .latest_state()?
                .storage(NONCE_PRECOMPILE_ADDRESS, slot.into())
                .map_err(Self::Error::from_eth_err)?
                .unwrap_or_default()
                .saturating_to();
            // Pending transactions on a lane form a gap-free sequence.
            let highest_pending = this
                .pool()
                .get_pending_transactions_by_address_and_nonce_key(from, nonce_key)
                .iter()
                .map(|tx| tx.nonce())
                .max();
            match highest_pending {
                Some(pending) if pending >= on_chain_nonce => {
                    pending.checked_add(1).ok_or_else(|| {
                        EthApiError::InvalidTransaction(RpcInvalidTransactionError::NonceMaxValue)
                    })
                }
                _ => Ok(on_chain_nonce),
            }
            .map_err(Self::Error::from)
        })
        .await
    }
}

impl<N: TempoEthApiBounds> EthState for AnvilTempoEthApi<N> {
    fn max_proof_window(&self) -> u64 {
        self.inner.eth_proof_window()
    }
}

impl<N: TempoEthApiBounds> EthFees for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> Trace for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> EthCall for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> GetBlockAccessList for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> Call for AnvilTempoEthApi<N> {
    fn call_gas_limit(&self) -> u64 {
        self.inner.gas_cap()
    }

    fn max_simulate_blocks(&self) -> u64 {
        self.inner.max_simulate_blocks()
    }

    fn compute_state_root_for_eth_simulate(&self) -> bool {
        self.inner.compute_state_root_for_eth_simulate()
    }

    fn evm_memory_limit(&self) -> u64 {
        self.inner.evm_memory_limit()
    }

    /// Returns the gas the fee payer can pay for, in its fee token.
    fn caller_gas_allowance(
        &self,
        mut db: impl Database<Error: Into<EthApiError>>,
        evm_env: &EvmEnvFor<Self::Evm>,
        tx_env: &TxEnvFor<Self::Evm>,
    ) -> Result<u64, Self::Error> {
        let fee_payer = tx_env.fee_payer().map_err(EVMError::<ProviderError, _>::from)?;
        let actions = StorageActions::disabled();
        let fee_token = self
            .evm_config()
            .resolve_fee_token(&mut db, tx_env, fee_payer, evm_env.cfg_env.spec, actions.clone())
            .map_err(ProviderError::other)?;
        let balance = db
            .get_token_balance(fee_token, fee_payer, evm_env.cfg_env.spec, actions)
            .map_err(ProviderError::other)?;
        Ok(balance
            .saturating_mul(TEMPO_GAS_PRICE_SCALING_FACTOR)
            .checked_div(U256::from(tx_env.inner.gas_price))
            .unwrap_or_default()
            .saturating_to())
    }

    fn create_txn_env(
        &self,
        evm_env: &EvmEnvFor<Self::Evm>,
        mut request: TempoTransactionRequest,
        mut db: impl Database<Error: Into<EthApiError>>,
    ) -> Result<TxEnvFor<Self::Evm>, Self::Error> {
        if let Some(nonce_key) = request.nonce_key.filter(|key| !key.is_zero())
            && request.nonce.is_none()
        {
            let nonce = if nonce_key == TEMPO_EXPIRING_NONCE_KEY {
                0
            } else {
                let from = request.from.unwrap_or_default();
                let slot = NonceManager::new().nonces[from][nonce_key].slot();
                db.storage(NONCE_PRECOMPILE_ADDRESS, slot).map_err(Into::into)?.saturating_to()
            };
            request.nonce = Some(nonce);
        }
        let expiring = request.nonce_key == Some(TEMPO_EXPIRING_NONCE_KEY);
        let hash = simulated_tx_hash(&request);
        let mut tx_env = self.inner.create_txn_env(evm_env, request, db)?;
        // Tempo keys an expiring nonce by the transaction hash, and from T1B on by the unique
        // identifier, which simulations otherwise share. Other simulations keep Tempo's
        // identifier, which also derives their channel ids.
        if let Some(aa) = tx_env.tempo_tx_env.as_mut()
            && aa.tx_hash.is_zero()
        {
            aa.tx_hash = hash;
        }
        if expiring {
            tx_env.unique_tx_identifier = Some(hash);
        }
        Ok(tx_env)
    }
}

/// Returns the hash a simulated Tempo transaction runs with: the hash of its request, so two
/// different calls in one bundle do not share an expiring nonce replay entry.
fn simulated_tx_hash(request: &TempoTransactionRequest) -> B256 {
    keccak256(serde_json::to_vec(request).unwrap_or_default())
}

impl<N: TempoEthApiBounds> EstimateCall for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> EthSubscriptions for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> LoadBlock for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> LoadReceipt for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> EthBlocks for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> LoadTransaction for AnvilTempoEthApi<N> {}

impl<N: TempoEthApiBounds> EthTransactions for AnvilTempoEthApi<N> {
    fn signers(&self) -> &SignersForRpc<Self::Provider, Self::NetworkTypes> {
        self.inner.signers()
    }

    fn send_raw_transaction_sync_timeout(&self) -> std::time::Duration {
        self.inner.send_raw_transaction_sync_timeout()
    }

    fn send_pool_transaction(
        &self,
        origin: TransactionOrigin,
        tx: WithEncoded<PoolTx<Self::Pool>>,
    ) -> impl Future<Output = Result<B256, Self::Error>> + Send {
        self.inner.send_pool_transaction(origin, tx).map_err(Into::into)
    }
}
