use super::{EthCallMany, FilterPollerBuilder};
use crate::{
    heart::PendingTransactionError,
    utils::{Eip1559Estimation, EstimatorFunction},
    EthCall, PendingTransaction, PendingTransactionBuilder, PendingTransactionConfig, Provider,
    ProviderCall, RootProvider, RpcWithBlock, SendableTx,
};
use alloy_network::{Ethereum, Network};
use alloy_network_primitives::BlockTransactionsKind;
use alloy_primitives::{
    Address, BlockHash, BlockNumber, Bytes, StorageKey, StorageValue, TxHash, B256, U128, U256, U64,
};
use alloy_rpc_client::{ClientRef, NoParams, WeakClient};
use alloy_rpc_types_eth::{
    simulate::{SimulatePayload, SimulatedBlock},
    AccessListResult, BlockId, BlockNumberOrTag, Bundle, EIP1186AccountProofResponse,
    EthCallResponse, FeeHistory, Filter, FilterChanges, Index, Log, SyncStatus,
};
use alloy_transport::TransportResult;
use serde_json::value::RawValue;
use std::{borrow::Cow, sync::Arc};

/// A wrapper struct around a type erased [`Provider`].
///
/// This type will delegate all functions to the wrapped provider, with the exception of non
/// object-safe functions (e.g. [`Provider::subscribe`]) which use the default trait implementation.
///
/// This is a convenience type for `Arc<dyn Provider<N> + 'static>`.
#[derive(Clone)]
pub struct DynProvider<N = Ethereum>(Arc<dyn Provider<N> + 'static>);

impl<N: Network> DynProvider<N> {
    /// Creates a new [`DynProvider`] by erasing the type.
    pub fn new<P>(provider: P) -> Self
    where
        P: Provider<N> + 'static,
    {
        Self(Arc::new(provider))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<N: Network> Provider<N> for DynProvider<N> {
    fn root(&self) -> &RootProvider<N> {
        self.0.root()
    }

    fn client(&self) -> ClientRef<'_> {
        self.0.client()
    }

    fn weak_client(&self) -> WeakClient {
        self.0.weak_client()
    }

    #[allow(clippy::use_self)]
    fn erased(self) -> DynProvider<N>
    where
        Self: Sized + 'static,
    {
        self
    }

    fn get_accounts(&self) -> ProviderCall<NoParams, Vec<Address>> {
        self.0.get_accounts()
    }

    fn get_blob_base_fee(&self) -> ProviderCall<NoParams, U128, u128> {
        self.0.get_blob_base_fee()
    }

    fn get_block_number(&self) -> ProviderCall<NoParams, U64, BlockNumber> {
        self.0.get_block_number()
    }

    fn call<'req>(&self, tx: &'req N::TransactionRequest) -> EthCall<'req, N, Bytes> {
        self.0.call(tx)
    }

    fn call_many<'req>(
        &self,
        bundles: &'req Vec<Bundle>,
    ) -> EthCallMany<'req, N, Vec<Vec<EthCallResponse>>> {
        self.0.call_many(bundles)
    }

    fn simulate<'req>(
        &self,
        payload: &'req SimulatePayload,
    ) -> RpcWithBlock<&'req SimulatePayload, Vec<SimulatedBlock<N::BlockResponse>>> {
        self.0.simulate(payload)
    }

    fn get_chain_id(&self) -> ProviderCall<NoParams, U64, u64> {
        self.0.get_chain_id()
    }

    fn create_access_list<'a>(
        &self,
        request: &'a N::TransactionRequest,
    ) -> RpcWithBlock<&'a N::TransactionRequest, AccessListResult> {
        self.0.create_access_list(request)
    }

    fn estimate_gas<'req>(&self, tx: &'req N::TransactionRequest) -> EthCall<'req, N, U64, u64> {
        self.0.estimate_gas(tx)
    }

    async fn estimate_eip1559_fees(
        &self,
        estimator: Option<EstimatorFunction>,
    ) -> TransportResult<Eip1559Estimation> {
        self.0.estimate_eip1559_fees(estimator).await
    }

    async fn get_fee_history(
        &self,
        block_count: u64,
        last_block: BlockNumberOrTag,
        reward_percentiles: &[f64],
    ) -> TransportResult<FeeHistory> {
        self.0.get_fee_history(block_count, last_block, reward_percentiles).await
    }

    fn get_gas_price(&self) -> ProviderCall<NoParams, U128, u128> {
        self.0.get_gas_price()
    }

    fn get_account(&self, address: Address) -> RpcWithBlock<Address, alloy_consensus::Account> {
        self.0.get_account(address)
    }

    fn get_balance(&self, address: Address) -> RpcWithBlock<Address, U256, U256> {
        self.0.get_balance(address)
    }

    async fn get_block(
        &self,
        block: BlockId,
        kind: BlockTransactionsKind,
    ) -> TransportResult<Option<N::BlockResponse>> {
        self.0.get_block(block, kind).await
    }

    async fn get_block_by_hash(
        &self,
        hash: BlockHash,
        kind: BlockTransactionsKind,
    ) -> TransportResult<Option<N::BlockResponse>> {
        self.0.get_block_by_hash(hash, kind).await
    }

    async fn get_block_by_number(
        &self,
        number: BlockNumberOrTag,
        kind: BlockTransactionsKind,
    ) -> TransportResult<Option<N::BlockResponse>> {
        self.0.get_block_by_number(number, kind).await
    }

    async fn get_block_transaction_count_by_hash(
        &self,
        hash: BlockHash,
    ) -> TransportResult<Option<u64>> {
        self.0.get_block_transaction_count_by_hash(hash).await
    }

    async fn get_block_transaction_count_by_number(
        &self,
        block_number: BlockNumberOrTag,
    ) -> TransportResult<Option<u64>> {
        self.0.get_block_transaction_count_by_number(block_number).await
    }

    fn get_block_receipts(
        &self,
        block: BlockId,
    ) -> ProviderCall<(BlockId,), Option<Vec<N::ReceiptResponse>>> {
        self.0.get_block_receipts(block)
    }

    fn get_code_at(&self, address: Address) -> RpcWithBlock<Address, Bytes> {
        self.0.get_code_at(address)
    }

    async fn watch_blocks(&self) -> TransportResult<FilterPollerBuilder<B256>> {
        self.0.watch_blocks().await
    }

    async fn watch_pending_transactions(&self) -> TransportResult<FilterPollerBuilder<B256>> {
        self.0.watch_pending_transactions().await
    }

    async fn watch_logs(&self, filter: &Filter) -> TransportResult<FilterPollerBuilder<Log>> {
        self.0.watch_logs(filter).await
    }

    async fn watch_full_pending_transactions(
        &self,
    ) -> TransportResult<FilterPollerBuilder<N::TransactionResponse>> {
        self.0.watch_full_pending_transactions().await
    }

    async fn get_filter_changes_dyn(&self, id: U256) -> TransportResult<FilterChanges> {
        self.0.get_filter_changes_dyn(id).await
    }

    async fn get_filter_logs(&self, id: U256) -> TransportResult<Vec<Log>> {
        self.0.get_filter_logs(id).await
    }

    async fn uninstall_filter(&self, id: U256) -> TransportResult<bool> {
        self.0.uninstall_filter(id).await
    }

    async fn watch_pending_transaction(
        &self,
        config: PendingTransactionConfig,
    ) -> Result<PendingTransaction, PendingTransactionError> {
        self.0.watch_pending_transaction(config).await
    }

    async fn get_logs(&self, filter: &Filter) -> TransportResult<Vec<Log>> {
        self.0.get_logs(filter).await
    }

    fn get_proof(
        &self,
        address: Address,
        keys: Vec<StorageKey>,
    ) -> RpcWithBlock<(Address, Vec<StorageKey>), EIP1186AccountProofResponse> {
        self.0.get_proof(address, keys)
    }

    fn get_storage_at(
        &self,
        address: Address,
        key: U256,
    ) -> RpcWithBlock<(Address, U256), StorageValue> {
        self.0.get_storage_at(address, key)
    }

    fn get_transaction_by_hash(
        &self,
        hash: TxHash,
    ) -> ProviderCall<(TxHash,), Option<N::TransactionResponse>> {
        self.0.get_transaction_by_hash(hash)
    }

    fn get_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: usize,
    ) -> ProviderCall<(B256, Index), Option<N::TransactionResponse>> {
        self.0.get_transaction_by_block_hash_and_index(block_hash, index)
    }

    fn get_raw_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: usize,
    ) -> ProviderCall<(B256, Index), Option<Bytes>> {
        self.0.get_raw_transaction_by_block_hash_and_index(block_hash, index)
    }

    fn get_transaction_by_block_number_and_index(
        &self,
        block_number: BlockNumberOrTag,
        index: usize,
    ) -> ProviderCall<(BlockNumberOrTag, Index), Option<N::TransactionResponse>> {
        self.0.get_transaction_by_block_number_and_index(block_number, index)
    }

    fn get_raw_transaction_by_block_number_and_index(
        &self,
        block_number: BlockNumberOrTag,
        index: usize,
    ) -> ProviderCall<(BlockNumberOrTag, Index), Option<Bytes>> {
        self.0.get_raw_transaction_by_block_number_and_index(block_number, index)
    }

    fn get_raw_transaction_by_hash(&self, hash: TxHash) -> ProviderCall<(TxHash,), Option<Bytes>> {
        self.0.get_raw_transaction_by_hash(hash)
    }

    fn get_transaction_count(
        &self,
        address: Address,
    ) -> RpcWithBlock<Address, U64, u64, fn(U64) -> u64> {
        self.0.get_transaction_count(address)
    }

    fn get_transaction_receipt(
        &self,
        hash: TxHash,
    ) -> ProviderCall<(TxHash,), Option<N::ReceiptResponse>> {
        self.0.get_transaction_receipt(hash)
    }

    async fn get_uncle(&self, tag: BlockId, idx: u64) -> TransportResult<Option<N::BlockResponse>> {
        self.0.get_uncle(tag, idx).await
    }

    async fn get_uncle_count(&self, tag: BlockId) -> TransportResult<u64> {
        self.0.get_uncle_count(tag).await
    }

    fn get_max_priority_fee_per_gas(&self) -> ProviderCall<NoParams, U128, u128> {
        self.0.get_max_priority_fee_per_gas()
    }

    async fn new_block_filter(&self) -> TransportResult<U256> {
        self.0.new_block_filter().await
    }

    async fn new_filter(&self, filter: &Filter) -> TransportResult<U256> {
        self.0.new_filter(filter).await
    }

    async fn new_pending_transactions_filter(&self, full: bool) -> TransportResult<U256> {
        self.0.new_pending_transactions_filter(full).await
    }

    async fn send_raw_transaction(
        &self,
        encoded_tx: &[u8],
    ) -> TransportResult<PendingTransactionBuilder<N>> {
        self.0.send_raw_transaction(encoded_tx).await
    }

    async fn send_transaction(
        &self,
        tx: N::TransactionRequest,
    ) -> TransportResult<PendingTransactionBuilder<N>> {
        self.0.send_transaction(tx).await
    }

    async fn send_tx_envelope(
        &self,
        tx: N::TxEnvelope,
    ) -> TransportResult<PendingTransactionBuilder<N>> {
        self.0.send_tx_envelope(tx).await
    }

    async fn send_transaction_internal(
        &self,
        tx: SendableTx<N>,
    ) -> TransportResult<PendingTransactionBuilder<N>> {
        self.0.send_transaction_internal(tx).await
    }

    #[cfg(feature = "pubsub")]
    async fn subscribe_blocks(
        &self,
    ) -> TransportResult<alloy_pubsub::Subscription<N::HeaderResponse>> {
        self.0.subscribe_blocks().await
    }

    #[cfg(feature = "pubsub")]
    async fn subscribe_pending_transactions(
        &self,
    ) -> TransportResult<alloy_pubsub::Subscription<B256>> {
        self.0.subscribe_pending_transactions().await
    }

    #[cfg(feature = "pubsub")]
    async fn subscribe_full_pending_transactions(
        &self,
    ) -> TransportResult<alloy_pubsub::Subscription<N::TransactionResponse>> {
        self.0.subscribe_full_pending_transactions().await
    }

    #[cfg(feature = "pubsub")]
    async fn subscribe_logs(
        &self,
        filter: &Filter,
    ) -> TransportResult<alloy_pubsub::Subscription<Log>> {
        self.0.subscribe_logs(filter).await
    }

    #[cfg(feature = "pubsub")]
    async fn unsubscribe(&self, id: B256) -> TransportResult<()> {
        self.0.unsubscribe(id).await
    }

    fn syncing(&self) -> ProviderCall<NoParams, SyncStatus> {
        self.0.syncing()
    }

    fn get_client_version(&self) -> ProviderCall<NoParams, String> {
        self.0.get_client_version()
    }

    fn get_sha3(&self, data: &[u8]) -> ProviderCall<(String,), B256> {
        self.0.get_sha3(data)
    }

    fn get_net_version(&self) -> ProviderCall<NoParams, U64, u64> {
        self.0.get_net_version()
    }

    async fn raw_request_dyn(
        &self,
        method: Cow<'static, str>,
        params: &RawValue,
    ) -> TransportResult<Box<RawValue>> {
        self.0.raw_request_dyn(method, params).await
    }

    fn transaction_request(&self) -> N::TransactionRequest {
        self.0.transaction_request()
    }
}

impl<N> std::fmt::Debug for DynProvider<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DynProvider").field(&"<dyn Provider>").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderBuilder;
    fn assert_provider<P: Provider + Sized + Clone + Unpin + 'static>(_: P) {}

    #[test]
    fn test_erased_provider() {
        let provider =
            ProviderBuilder::new().on_http("http://localhost:8080".parse().unwrap()).erased();
        assert_provider(provider);
    }
}
