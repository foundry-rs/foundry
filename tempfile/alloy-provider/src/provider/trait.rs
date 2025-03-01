//! Ethereum JSON-RPC provider.

#![allow(unknown_lints, elided_named_lifetimes)]

use crate::{
    heart::PendingTransactionError,
    utils::{self, Eip1559Estimation, EstimatorFunction},
    EthCall, Identity, PendingTransaction, PendingTransactionBuilder, PendingTransactionConfig,
    ProviderBuilder, ProviderCall, RootProvider, RpcWithBlock, SendableTx,
};
use alloy_consensus::BlockHeader;
use alloy_eips::eip2718::Encodable2718;
use alloy_json_rpc::{RpcError, RpcRecv, RpcSend};
use alloy_network::{Ethereum, Network};
use alloy_network_primitives::{BlockResponse, BlockTransactionsKind, ReceiptResponse};
use alloy_primitives::{
    hex, Address, BlockHash, BlockNumber, Bytes, StorageKey, StorageValue, TxHash, B256, U128,
    U256, U64,
};
use alloy_rpc_client::{ClientRef, NoParams, PollerBuilder, WeakClient};
use alloy_rpc_types_eth::{
    simulate::{SimulatePayload, SimulatedBlock},
    AccessListResult, BlockId, BlockNumberOrTag, Bundle, EIP1186AccountProofResponse,
    EthCallResponse, FeeHistory, Filter, FilterChanges, Index, Log, SyncStatus,
};
use alloy_transport::TransportResult;
use serde_json::value::RawValue;
use std::borrow::Cow;

use super::{DynProvider, Empty, EthCallMany, MulticallBuilder};

/// A task that polls the provider with `eth_getFilterChanges`, returning a list of `R`.
///
/// See [`PollerBuilder`] for more details.
pub type FilterPollerBuilder<R> = PollerBuilder<(U256,), Vec<R>>;

/// Ethereum JSON-RPC interface.
///
/// # Subscriptions
///
/// The provider supports `pubsub` subscriptions to new block headers and
/// pending transactions. This is only available on `pubsub` clients, such as
/// Websockets or IPC.
///
/// For a polling alternatives available over HTTP, use the `watch_*` methods.
/// However, be aware that polling increases RPC usage drastically.
///
/// ## Special treatment of EIP-1559
///
/// While many RPC features are encapsulated by traits like [`DebugApi`],
/// EIP-1559 fee estimation is generally assumed to be on by default. We
/// generally assume that EIP-1559 is supported by the client and will
/// proactively use it by default.
///
/// As a result, the provider supports EIP-1559 fee estimation the ethereum
/// [`TransactionBuilder`] will use it by default. We acknowledge that this
/// means EIP-1559 has a privileged status in comparison to other transaction
/// types. Networks that DO NOT support EIP-1559 should create their own
/// [`TransactionBuilder`] and Fillers to change this behavior.
///
/// [`TransactionBuilder`]: alloy_network::TransactionBuilder
/// [`DebugApi`]: crate::ext::DebugApi
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[auto_impl::auto_impl(&, &mut, Rc, Arc, Box)]
pub trait Provider<N: Network = Ethereum>: Send + Sync {
    /// Returns the root provider.
    fn root(&self) -> &RootProvider<N>;

    /// Returns the [`ProviderBuilder`](crate::ProviderBuilder) to build on.
    fn builder() -> ProviderBuilder<Identity, Identity, N>
    where
        Self: Sized,
    {
        ProviderBuilder::default()
    }

    /// Returns the RPC client used to send requests.
    ///
    /// NOTE: this method should not be overridden.
    #[inline]
    fn client(&self) -> ClientRef<'_> {
        self.root().client()
    }

    /// Returns a [`Weak`](std::sync::Weak) RPC client used to send requests.
    ///
    /// NOTE: this method should not be overridden.
    #[inline]
    fn weak_client(&self) -> WeakClient {
        self.root().weak_client()
    }

    /// Returns a type erased provider wrapped in Arc [`super::DynProvider`].
    ///
    /// ```no_run
    /// use alloy_provider::{DynProvider, Provider, ProviderBuilder};
    /// # async fn f() {
    /// let provider: DynProvider =
    ///     ProviderBuilder::new().on_http("http://localhost:8080".parse().unwrap()).erased();
    /// let block = provider.get_block_number().await.unwrap();
    ///
    /// # }
    /// ```
    #[auto_impl(keep_default_for(&, &mut, Rc, Arc, Box))]
    fn erased(self) -> DynProvider<N>
    where
        Self: Sized + 'static,
    {
        DynProvider::new(self)
    }

    /// Gets the accounts in the remote node. This is usually empty unless you're using a local
    /// node.
    fn get_accounts(&self) -> ProviderCall<NoParams, Vec<Address>> {
        self.client().request_noparams("eth_accounts").into()
    }

    /// Returns the base fee per blob gas (blob gas price) in wei.
    fn get_blob_base_fee(&self) -> ProviderCall<NoParams, U128, u128> {
        self.client()
            .request_noparams("eth_blobBaseFee")
            .map_resp(utils::convert_u128 as fn(U128) -> u128)
            .into()
    }

    /// Get the last block number available.
    fn get_block_number(&self) -> ProviderCall<NoParams, U64, BlockNumber> {
        self.client()
            .request_noparams("eth_blockNumber")
            .map_resp(utils::convert_u64 as fn(U64) -> u64)
            .into()
    }

    /// Execute a smart contract call with a transaction request and state
    /// overrides, without publishing a transaction.
    ///
    /// This function returns [`EthCall`] which can be used to execute the
    /// call, or to add [`StateOverride`] or a [`BlockId`]. If no overrides
    /// or block ID is provided, the call will be executed on the pending block
    /// with the current state.
    ///
    /// [`StateOverride`]: alloy_rpc_types_eth::state::StateOverride
    ///
    /// ## Example
    ///
    /// ```
    /// # use alloy_provider::Provider;
    /// # use alloy_eips::BlockId;
    /// # use alloy_rpc_types_eth::state::StateOverride;
    /// # use alloy_transport::BoxTransport;
    /// # async fn example<P: Provider>(
    /// #    provider: P,
    /// #    my_overrides: StateOverride
    /// # ) -> Result<(), Box<dyn std::error::Error>> {
    /// # let tx = alloy_rpc_types_eth::transaction::TransactionRequest::default();
    /// // Execute a call on the latest block, with no state overrides
    /// let output = provider.call(&tx).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "eth_call")]
    #[doc(alias = "call_with_overrides")]
    fn call<'req>(&self, tx: &'req N::TransactionRequest) -> EthCall<'req, N, Bytes> {
        EthCall::call(self.weak_client(), tx).block(BlockNumberOrTag::Pending.into())
    }

    /// Execute a list of [`Bundle`] against the provided [`StateContext`] and [`StateOverride`],
    /// without publishing a transaction.
    ///
    /// This function returns [`EthCallMany`] builder which is used to execute the call, and also
    /// set the [`StateContext`] and [`StateOverride`].
    ///
    /// [`StateContext`]: alloy_rpc_types_eth::StateContext
    /// [`StateOverride`]: alloy_rpc_types_eth::state::StateOverride
    #[doc(alias = "eth_callMany")]
    fn call_many<'req>(
        &self,
        bundles: &'req Vec<Bundle>,
    ) -> EthCallMany<'req, N, Vec<Vec<EthCallResponse>>> {
        EthCallMany::new(self.weak_client(), bundles)
    }

    /// Execute a multicall by leveraging the [`MulticallBuilder`].
    ///
    /// This function returns a [`MulticallBuilder`] which is used to add multiple calls and execute
    /// them.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// use alloy_primitives::address;
    /// use alloy_provider::{MulticallBuilder, Provider, ProviderBuilder};
    /// use alloy_sol_types::sol;
    ///
    /// sol! {
    ///    #[sol(rpc)]
    ///    #[derive(Debug, PartialEq)]
    ///    interface ERC20 {
    ///        function totalSupply() external view returns (uint256 totalSupply);
    ///        function balanceOf(address owner) external view returns (uint256 balance);
    ///    }
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    ///     let provider = ProviderBuilder::new().on_http("https://eth.merkle.io".parse().unwrap());
    ///     let erc20 = ERC20::new(weth, &provider);
    ///
    ///     let ts_call = erc20.totalSupply();
    ///     let balance_call = erc20.balanceOf(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045"));
    ///
    ///     let multicall = provider.multicall().add(ts_call).add(balance_call);
    ///
    ///     let (_block_num, (total_supply, balance)) = multicall.aggregate().await.unwrap();
    ///
    ///     println!("Total Supply: {:?}, Balance: {:?}", total_supply, balance);
    /// }
    /// ```
    #[auto_impl(keep_default_for(&, &mut, Rc, Arc, Box))]
    fn multicall(&self) -> MulticallBuilder<Empty, &Self, N>
    where
        Self: Sized,
    {
        MulticallBuilder::new(self)
    }

    /// Executes an arbitrary number of transactions on top of the requested state.
    ///
    /// The transactions are packed into individual blocks. Overrides can be provided.
    #[doc(alias = "eth_simulateV1")]
    fn simulate<'req>(
        &self,
        payload: &'req SimulatePayload,
    ) -> RpcWithBlock<&'req SimulatePayload, Vec<SimulatedBlock<N::BlockResponse>>> {
        self.client().request("eth_simulateV1", payload).into()
    }

    /// Gets the chain ID.
    fn get_chain_id(&self) -> ProviderCall<NoParams, U64, u64> {
        self.client()
            .request_noparams("eth_chainId")
            .map_resp(utils::convert_u64 as fn(U64) -> u64)
            .into()
    }

    /// Create an [EIP-2930] access list.
    ///
    /// [EIP-2930]: https://eips.ethereum.org/EIPS/eip-2930
    fn create_access_list<'a>(
        &self,
        request: &'a N::TransactionRequest,
    ) -> RpcWithBlock<&'a N::TransactionRequest, AccessListResult> {
        self.client().request("eth_createAccessList", request).into()
    }

    /// Create an [`EthCall`] future to estimate the gas required for a
    /// transaction.
    ///
    /// The future can be used to specify a [`StateOverride`] or [`BlockId`]
    /// before dispatching the call. If no overrides or block ID is provided,
    /// the gas estimate will be computed for the pending block with the
    /// current state.
    ///
    /// [`StateOverride`]: alloy_rpc_types_eth::state::StateOverride
    ///
    /// # Note
    ///
    /// Not all client implementations support state overrides for eth_estimateGas.
    fn estimate_gas<'req>(&self, tx: &'req N::TransactionRequest) -> EthCall<'req, N, U64, u64> {
        EthCall::gas_estimate(self.weak_client(), tx)
            .block(BlockNumberOrTag::Pending.into())
            .map_resp(utils::convert_u64)
    }

    /// Estimates the EIP1559 `maxFeePerGas` and `maxPriorityFeePerGas` fields.
    ///
    /// Receives an optional [EstimatorFunction] that can be used to modify
    /// how to estimate these fees.
    async fn estimate_eip1559_fees(
        &self,
        estimator: Option<EstimatorFunction>,
    ) -> TransportResult<Eip1559Estimation> {
        let fee_history = self
            .get_fee_history(
                utils::EIP1559_FEE_ESTIMATION_PAST_BLOCKS,
                BlockNumberOrTag::Latest,
                &[utils::EIP1559_FEE_ESTIMATION_REWARD_PERCENTILE],
            )
            .await?;

        // if the base fee of the Latest block is 0 then we need check if the latest block even has
        // a base fee/supports EIP1559
        let base_fee_per_gas = match fee_history.latest_block_base_fee() {
            Some(base_fee) if base_fee != 0 => base_fee,
            _ => {
                // empty response, fetch basefee from latest block directly
                self.get_block_by_number(BlockNumberOrTag::Latest, BlockTransactionsKind::Hashes)
                    .await?
                    .ok_or(RpcError::NullResp)?
                    .header()
                    .as_ref()
                    .base_fee_per_gas()
                    .ok_or(RpcError::UnsupportedFeature("eip1559"))?
                    .into()
            }
        };

        Ok(estimator.unwrap_or(utils::eip1559_default_estimator)(
            base_fee_per_gas,
            &fee_history.reward.unwrap_or_default(),
        ))
    }

    /// Returns a collection of historical gas information [FeeHistory] which
    /// can be used to calculate the EIP1559 fields `maxFeePerGas` and `maxPriorityFeePerGas`.
    /// `block_count` can range from 1 to 1024 blocks in a single request.
    async fn get_fee_history(
        &self,
        block_count: u64,
        last_block: BlockNumberOrTag,
        reward_percentiles: &[f64],
    ) -> TransportResult<FeeHistory> {
        self.client()
            .request("eth_feeHistory", (U64::from(block_count), last_block, reward_percentiles))
            .await
    }

    /// Gets the current gas price in wei.
    fn get_gas_price(&self) -> ProviderCall<NoParams, U128, u128> {
        self.client()
            .request_noparams("eth_gasPrice")
            .map_resp(utils::convert_u128 as fn(U128) -> u128)
            .into()
    }

    /// Retrieves account information ([Account](alloy_consensus::Account)) for the given [Address]
    /// at the particular [BlockId].
    fn get_account(&self, address: Address) -> RpcWithBlock<Address, alloy_consensus::Account> {
        self.client().request("eth_getAccount", address).into()
    }

    /// Gets the balance of the account.
    ///
    /// Defaults to the latest block. See also [`RpcWithBlock::block_id`].
    fn get_balance(&self, address: Address) -> RpcWithBlock<Address, U256, U256> {
        self.client().request("eth_getBalance", address).into()
    }

    /// Gets a block by either its hash, tag, or number, with full transactions or only hashes.
    async fn get_block(
        &self,
        block: BlockId,
        kind: BlockTransactionsKind,
    ) -> TransportResult<Option<N::BlockResponse>> {
        match block {
            BlockId::Hash(hash) => self.get_block_by_hash(hash.into(), kind).await,
            BlockId::Number(number) => self.get_block_by_number(number, kind).await,
        }
    }

    /// Gets a block by its [BlockHash], with full transactions or only hashes.
    async fn get_block_by_hash(
        &self,
        hash: BlockHash,
        kind: BlockTransactionsKind,
    ) -> TransportResult<Option<N::BlockResponse>> {
        let full = match kind {
            BlockTransactionsKind::Full => true,
            BlockTransactionsKind::Hashes => false,
        };

        let block = self
            .client()
            .request::<_, Option<N::BlockResponse>>("eth_getBlockByHash", (hash, full))
            .await?
            .map(|mut block| {
                if !full {
                    // this ensures an empty response for `Hashes` has the expected form
                    // this is required because deserializing [] is ambiguous
                    block.transactions_mut().convert_to_hashes();
                }
                block
            });

        Ok(block)
    }

    /// Get a block by its number.
    // TODO: Network associate
    async fn get_block_by_number(
        &self,
        number: BlockNumberOrTag,
        kind: BlockTransactionsKind,
    ) -> TransportResult<Option<N::BlockResponse>> {
        let full = match kind {
            BlockTransactionsKind::Full => true,
            BlockTransactionsKind::Hashes => false,
        };

        let block = self
            .client()
            .request::<_, Option<N::BlockResponse>>("eth_getBlockByNumber", (number, full))
            .await?
            .map(|mut block| {
                if !full {
                    // this ensures an empty response for `Hashes` has the expected form
                    // this is required because deserializing [] is ambiguous
                    block.transactions_mut().convert_to_hashes();
                }
                block
            });
        Ok(block)
    }

    /// Returns the number of transactions in a block from a block matching the given block hash.
    async fn get_block_transaction_count_by_hash(
        &self,
        hash: BlockHash,
    ) -> TransportResult<Option<u64>> {
        self.client()
            .request("eth_getBlockTransactionCountByHash", (hash,))
            .await
            .map(|opt_count: Option<U64>| opt_count.map(|count| count.to::<u64>()))
    }

    /// Returns the number of transactions in a block matching the given block number.
    async fn get_block_transaction_count_by_number(
        &self,
        block_number: BlockNumberOrTag,
    ) -> TransportResult<Option<u64>> {
        self.client()
            .request("eth_getBlockTransactionCountByNumber", (block_number,))
            .await
            .map(|opt_count: Option<U64>| opt_count.map(|count| count.to::<u64>()))
    }

    /// Gets the selected block [BlockId] receipts.
    fn get_block_receipts(
        &self,
        block: BlockId,
    ) -> ProviderCall<(BlockId,), Option<Vec<N::ReceiptResponse>>> {
        self.client().request("eth_getBlockReceipts", (block,)).into()
    }

    /// Gets the bytecode located at the corresponding [Address].
    fn get_code_at(&self, address: Address) -> RpcWithBlock<Address, Bytes> {
        self.client().request("eth_getCode", address).into()
    }

    /// Watch for new blocks by polling the provider with
    /// [`eth_getFilterChanges`](Self::get_filter_changes).
    ///
    /// Returns a builder that is used to configure the poller. See [`PollerBuilder`] for more
    /// details.
    ///
    /// # Examples
    ///
    /// Get the next 5 blocks:
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use futures::StreamExt;
    ///
    /// let poller = provider.watch_blocks().await?;
    /// let mut stream = poller.into_stream().flat_map(futures::stream::iter).take(5);
    /// while let Some(block_hash) = stream.next().await {
    ///    println!("new block: {block_hash}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn watch_blocks(&self) -> TransportResult<FilterPollerBuilder<B256>> {
        let id = self.new_block_filter().await?;
        Ok(PollerBuilder::new(self.weak_client(), "eth_getFilterChanges", (id,)))
    }

    /// Watch for new pending transaction by polling the provider with
    /// [`eth_getFilterChanges`](Self::get_filter_changes).
    ///
    /// Returns a builder that is used to configure the poller. See [`PollerBuilder`] for more
    /// details.
    ///
    /// # Examples
    ///
    /// Get the next 5 pending transaction hashes:
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use futures::StreamExt;
    ///
    /// let poller = provider.watch_pending_transactions().await?;
    /// let mut stream = poller.into_stream().flat_map(futures::stream::iter).take(5);
    /// while let Some(tx_hash) = stream.next().await {
    ///    println!("new pending transaction hash: {tx_hash}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn watch_pending_transactions(&self) -> TransportResult<FilterPollerBuilder<B256>> {
        let id = self.new_pending_transactions_filter(false).await?;
        Ok(PollerBuilder::new(self.weak_client(), "eth_getFilterChanges", (id,)))
    }

    /// Watch for new logs using the given filter by polling the provider with
    /// [`eth_getFilterChanges`](Self::get_filter_changes).
    ///
    /// Returns a builder that is used to configure the poller. See [`PollerBuilder`] for more
    /// details.
    ///
    /// # Examples
    ///
    /// Get the next 5 USDC transfer logs:
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use alloy_primitives::{address, b256};
    /// use alloy_rpc_types_eth::Filter;
    /// use futures::StreamExt;
    ///
    /// let address = address!("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
    /// let transfer_signature = b256!("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");
    /// let filter = Filter::new().address(address).event_signature(transfer_signature);
    ///
    /// let poller = provider.watch_logs(&filter).await?;
    /// let mut stream = poller.into_stream().flat_map(futures::stream::iter).take(5);
    /// while let Some(log) = stream.next().await {
    ///    println!("new log: {log:#?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn watch_logs(&self, filter: &Filter) -> TransportResult<FilterPollerBuilder<Log>> {
        let id = self.new_filter(filter).await?;
        Ok(PollerBuilder::new(self.weak_client(), "eth_getFilterChanges", (id,)))
    }

    /// Watch for new pending transaction bodies by polling the provider with
    /// [`eth_getFilterChanges`](Self::get_filter_changes).
    ///
    /// Returns a builder that is used to configure the poller. See [`PollerBuilder`] for more
    /// details.
    ///
    /// # Support
    ///
    /// This endpoint might not be supported by all clients.
    ///
    /// # Examples
    ///
    /// Get the next 5 pending transaction bodies:
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use futures::StreamExt;
    ///
    /// let poller = provider.watch_full_pending_transactions().await?;
    /// let mut stream = poller.into_stream().flat_map(futures::stream::iter).take(5);
    /// while let Some(tx) = stream.next().await {
    ///    println!("new pending transaction: {tx:#?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn watch_full_pending_transactions(
        &self,
    ) -> TransportResult<FilterPollerBuilder<N::TransactionResponse>> {
        let id = self.new_pending_transactions_filter(true).await?;
        Ok(PollerBuilder::new(self.weak_client(), "eth_getFilterChanges", (id,)))
    }

    /// Get a list of values that have been added since the last poll.
    ///
    /// The return value depends on what stream `id` corresponds to.
    /// See [`FilterChanges`] for all possible return values.
    #[auto_impl(keep_default_for(&, &mut, Rc, Arc, Box))]
    async fn get_filter_changes<R: RpcRecv>(&self, id: U256) -> TransportResult<Vec<R>>
    where
        Self: Sized,
    {
        self.client().request("eth_getFilterChanges", (id,)).await
    }

    /// Get a list of values that have been added since the last poll.
    ///
    /// This returns an enum over all possible return values. You probably want to use
    /// [`get_filter_changes`](Self::get_filter_changes) instead.
    async fn get_filter_changes_dyn(&self, id: U256) -> TransportResult<FilterChanges> {
        self.client().request("eth_getFilterChanges", (id,)).await
    }

    /// Retrieves a [`Vec<Log>`] for the given filter ID.
    async fn get_filter_logs(&self, id: U256) -> TransportResult<Vec<Log>> {
        self.client().request("eth_getFilterLogs", (id,)).await
    }

    /// Request provider to uninstall the filter with the given ID.
    async fn uninstall_filter(&self, id: U256) -> TransportResult<bool> {
        self.client().request("eth_uninstallFilter", (id,)).await
    }

    /// Watch for the confirmation of a single pending transaction with the given configuration.
    ///
    /// Note that this is handled internally rather than calling any specific RPC method, and as
    /// such should not be overridden.
    #[inline]
    async fn watch_pending_transaction(
        &self,
        config: PendingTransactionConfig,
    ) -> Result<PendingTransaction, PendingTransactionError> {
        self.root().watch_pending_transaction(config).await
    }

    /// Retrieves a [`Vec<Log>`] with the given [Filter].
    async fn get_logs(&self, filter: &Filter) -> TransportResult<Vec<Log>> {
        self.client().request("eth_getLogs", (filter,)).await
    }

    /// Get the account and storage values of the specified account including the merkle proofs.
    ///
    /// This call can be used to verify that the data has not been tampered with.
    fn get_proof(
        &self,
        address: Address,
        keys: Vec<StorageKey>,
    ) -> RpcWithBlock<(Address, Vec<StorageKey>), EIP1186AccountProofResponse> {
        self.client().request("eth_getProof", (address, keys)).into()
    }

    /// Gets the specified storage value from [Address].
    fn get_storage_at(
        &self,
        address: Address,
        key: U256,
    ) -> RpcWithBlock<(Address, U256), StorageValue> {
        self.client().request("eth_getStorageAt", (address, key)).into()
    }

    /// Gets a transaction by its [TxHash].
    fn get_transaction_by_hash(
        &self,
        hash: TxHash,
    ) -> ProviderCall<(TxHash,), Option<N::TransactionResponse>> {
        self.client().request("eth_getTransactionByHash", (hash,)).into()
    }

    /// Gets a transaction by block hash and transaction index position.
    fn get_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: usize,
    ) -> ProviderCall<(B256, Index), Option<N::TransactionResponse>> {
        self.client()
            .request("eth_getTransactionByBlockHashAndIndex", (block_hash, Index(index)))
            .into()
    }

    /// Gets a raw transaction by block hash and transaction index position.
    fn get_raw_transaction_by_block_hash_and_index(
        &self,
        block_hash: B256,
        index: usize,
    ) -> ProviderCall<(B256, Index), Option<Bytes>> {
        self.client()
            .request("eth_getRawTransactionByBlockHashAndIndex", (block_hash, Index(index)))
            .into()
    }

    /// Gets a transaction by block number and transaction index position.
    fn get_transaction_by_block_number_and_index(
        &self,
        block_number: BlockNumberOrTag,
        index: usize,
    ) -> ProviderCall<(BlockNumberOrTag, Index), Option<N::TransactionResponse>> {
        self.client()
            .request("eth_getTransactionByBlockNumberAndIndex", (block_number, Index(index)))
            .into()
    }

    /// Gets a raw transaction by block number and transaction index position.
    fn get_raw_transaction_by_block_number_and_index(
        &self,
        block_number: BlockNumberOrTag,
        index: usize,
    ) -> ProviderCall<(BlockNumberOrTag, Index), Option<Bytes>> {
        self.client()
            .request("eth_getRawTransactionByBlockNumberAndIndex", (block_number, Index(index)))
            .into()
    }

    /// Returns the EIP-2718 encoded transaction if it exists, see also
    /// [Decodable2718](alloy_eips::eip2718::Decodable2718).
    ///
    /// If the transaction is an EIP-4844 transaction that is still in the pool (pending) it will
    /// include the sidecar, otherwise it will the consensus variant without the sidecar:
    /// [TxEip4844](alloy_consensus::transaction::eip4844::TxEip4844).
    ///
    /// This can be decoded into [TxEnvelope](alloy_consensus::transaction::TxEnvelope).
    fn get_raw_transaction_by_hash(&self, hash: TxHash) -> ProviderCall<(TxHash,), Option<Bytes>> {
        self.client().request("eth_getRawTransactionByHash", (hash,)).into()
    }

    /// Gets the transaction count (AKA "nonce") of the corresponding address.
    #[doc(alias = "get_nonce")]
    #[doc(alias = "get_account_nonce")]
    fn get_transaction_count(
        &self,
        address: Address,
    ) -> RpcWithBlock<Address, U64, u64, fn(U64) -> u64> {
        self.client()
            .request("eth_getTransactionCount", address)
            .map_resp(utils::convert_u64 as fn(U64) -> u64)
            .into()
    }

    /// Gets a transaction receipt if it exists, by its [TxHash].
    fn get_transaction_receipt(
        &self,
        hash: TxHash,
    ) -> ProviderCall<(TxHash,), Option<N::ReceiptResponse>> {
        self.client().request("eth_getTransactionReceipt", (hash,)).into()
    }

    /// Gets an uncle block through the tag [BlockId] and index [u64].
    async fn get_uncle(&self, tag: BlockId, idx: u64) -> TransportResult<Option<N::BlockResponse>> {
        let idx = U64::from(idx);
        match tag {
            BlockId::Hash(hash) => {
                self.client()
                    .request("eth_getUncleByBlockHashAndIndex", (hash.block_hash, idx))
                    .await
            }
            BlockId::Number(number) => {
                self.client().request("eth_getUncleByBlockNumberAndIndex", (number, idx)).await
            }
        }
    }

    /// Gets the number of uncles for the block specified by the tag [BlockId].
    async fn get_uncle_count(&self, tag: BlockId) -> TransportResult<u64> {
        match tag {
            BlockId::Hash(hash) => self
                .client()
                .request("eth_getUncleCountByBlockHash", (hash.block_hash,))
                .await
                .map(|count: U64| count.to::<u64>()),
            BlockId::Number(number) => self
                .client()
                .request("eth_getUncleCountByBlockNumber", (number,))
                .await
                .map(|count: U64| count.to::<u64>()),
        }
    }

    /// Returns a suggestion for the current `maxPriorityFeePerGas` in wei.
    fn get_max_priority_fee_per_gas(&self) -> ProviderCall<NoParams, U128, u128> {
        self.client()
            .request_noparams("eth_maxPriorityFeePerGas")
            .map_resp(utils::convert_u128 as fn(U128) -> u128)
            .into()
    }

    /// Notify the provider that we are interested in new blocks.
    ///
    /// Returns the ID to use with [`eth_getFilterChanges`](Self::get_filter_changes).
    ///
    /// See also [`watch_blocks`](Self::watch_blocks) to configure a poller.
    async fn new_block_filter(&self) -> TransportResult<U256> {
        self.client().request_noparams("eth_newBlockFilter").await
    }

    /// Notify the provider that we are interested in logs that match the given filter.
    ///
    /// Returns the ID to use with [`eth_getFilterChanges`](Self::get_filter_changes).
    ///
    /// See also [`watch_logs`](Self::watch_logs) to configure a poller.
    async fn new_filter(&self, filter: &Filter) -> TransportResult<U256> {
        self.client().request("eth_newFilter", (filter,)).await
    }

    /// Notify the provider that we are interested in new pending transactions.
    ///
    /// If `full` is `true`, the stream will consist of full transaction bodies instead of just the
    /// hashes. This not supported by all clients.
    ///
    /// Returns the ID to use with [`eth_getFilterChanges`](Self::get_filter_changes).
    ///
    /// See also [`watch_pending_transactions`](Self::watch_pending_transactions) to configure a
    /// poller.
    async fn new_pending_transactions_filter(&self, full: bool) -> TransportResult<U256> {
        // NOTE: We don't want to send `false` as the client might not support it.
        let param = if full { &[true][..] } else { &[] };
        self.client().request("eth_newPendingTransactionFilter", param).await
    }

    /// Broadcasts a raw transaction RLP bytes to the network.
    ///
    /// See [`send_transaction`](Self::send_transaction) for more details.
    async fn send_raw_transaction(
        &self,
        encoded_tx: &[u8],
    ) -> TransportResult<PendingTransactionBuilder<N>> {
        let rlp_hex = hex::encode_prefixed(encoded_tx);
        let tx_hash = self.client().request("eth_sendRawTransaction", (rlp_hex,)).await?;
        Ok(PendingTransactionBuilder::new(self.root().clone(), tx_hash))
    }

    /// Broadcasts a transaction to the network.
    ///
    /// Returns a [`PendingTransactionBuilder`] which can be used to configure
    /// how and when to await the transaction's confirmation.
    ///
    /// # Examples
    ///
    /// See [`PendingTransactionBuilder`](crate::PendingTransactionBuilder) for more examples.
    ///
    /// ```no_run
    /// # async fn example<N: alloy_network::Network>(provider: impl alloy_provider::Provider, tx: alloy_rpc_types_eth::transaction::TransactionRequest) -> Result<(), Box<dyn std::error::Error>> {
    /// let tx_hash = provider.send_transaction(tx)
    ///     .await?
    ///     .with_required_confirmations(2)
    ///     .with_timeout(Some(std::time::Duration::from_secs(60)))
    ///     .watch()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn send_transaction(
        &self,
        tx: N::TransactionRequest,
    ) -> TransportResult<PendingTransactionBuilder<N>> {
        self.send_transaction_internal(SendableTx::Builder(tx)).await
    }

    /// Broadcasts a transaction envelope to the network.
    ///
    /// Returns a [`PendingTransactionBuilder`] which can be used to configure
    /// how and when to await the transaction's confirmation.
    async fn send_tx_envelope(
        &self,
        tx: N::TxEnvelope,
    ) -> TransportResult<PendingTransactionBuilder<N>> {
        self.send_transaction_internal(SendableTx::Envelope(tx)).await
    }

    /// This method allows [`ProviderLayer`] and [`TxFiller`] to build the
    /// transaction and send it to the network without changing user-facing
    /// APIs. Generally implementors should NOT override this method.
    ///
    /// [`send_transaction`]: Self::send_transaction
    /// [`ProviderLayer`]: crate::ProviderLayer
    /// [`TxFiller`]: crate::TxFiller
    #[doc(hidden)]
    async fn send_transaction_internal(
        &self,
        tx: SendableTx<N>,
    ) -> TransportResult<PendingTransactionBuilder<N>> {
        // Make sure to initialize heartbeat before we submit transaction, so that
        // we don't miss it if user will subscriber to it immediately after sending.
        let _handle = self.root().get_heart();

        match tx {
            SendableTx::Builder(mut tx) => {
                alloy_network::TransactionBuilder::prep_for_submission(&mut tx);
                let tx_hash = self.client().request("eth_sendTransaction", (tx,)).await?;
                Ok(PendingTransactionBuilder::new(self.root().clone(), tx_hash))
            }
            SendableTx::Envelope(tx) => {
                let encoded_tx = tx.encoded_2718();
                self.send_raw_transaction(&encoded_tx).await
            }
        }
    }

    /// Subscribe to a stream of new block headers.
    ///
    /// # Errors
    ///
    /// This method is only available on `pubsub` clients, such as WebSockets or IPC, and will
    /// return a [`PubsubUnavailable`](alloy_transport::TransportErrorKind::PubsubUnavailable)
    /// transport error if the client does not support it.
    ///
    /// For a polling alternative available over HTTP, use [`Provider::watch_blocks`].
    /// However, be aware that polling increases RPC usage drastically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use futures::StreamExt;
    ///
    /// let sub = provider.subscribe_blocks().await?;
    /// let mut stream = sub.into_stream().take(5);
    /// while let Some(block) = stream.next().await {
    ///    println!("new block: {block:#?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "pubsub")]
    async fn subscribe_blocks(
        &self,
    ) -> TransportResult<alloy_pubsub::Subscription<N::HeaderResponse>> {
        self.root().pubsub_frontend()?;
        let id = self.client().request("eth_subscribe", ("newHeads",)).await?;
        self.root().get_subscription(id).await
    }

    /// Subscribe to a stream of pending transaction hashes.
    ///
    /// # Errors
    ///
    /// This method is only available on `pubsub` clients, such as WebSockets or IPC, and will
    /// return a [`PubsubUnavailable`](alloy_transport::TransportErrorKind::PubsubUnavailable)
    /// transport error if the client does not support it.
    ///
    /// For a polling alternative available over HTTP, use [`Provider::watch_pending_transactions`].
    /// However, be aware that polling increases RPC usage drastically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use futures::StreamExt;
    ///
    /// let sub = provider.subscribe_pending_transactions().await?;
    /// let mut stream = sub.into_stream().take(5);
    /// while let Some(tx_hash) = stream.next().await {
    ///    println!("new pending transaction hash: {tx_hash}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "pubsub")]
    async fn subscribe_pending_transactions(
        &self,
    ) -> TransportResult<alloy_pubsub::Subscription<B256>> {
        self.root().pubsub_frontend()?;
        let id = self.client().request("eth_subscribe", ("newPendingTransactions",)).await?;
        self.root().get_subscription(id).await
    }

    /// Subscribe to a stream of pending transaction bodies.
    ///
    /// # Support
    ///
    /// This endpoint is compatible only with Geth client version 1.11.0 or later.
    ///
    /// # Errors
    ///
    /// This method is only available on `pubsub` clients, such as WebSockets or IPC, and will
    /// return a [`PubsubUnavailable`](alloy_transport::TransportErrorKind::PubsubUnavailable)
    /// transport error if the client does not support it.
    ///
    /// For a polling alternative available over HTTP, use
    /// [`Provider::watch_full_pending_transactions`]. However, be aware that polling increases
    /// RPC usage drastically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use futures::StreamExt;
    ///
    /// let sub = provider.subscribe_full_pending_transactions().await?;
    /// let mut stream = sub.into_stream().take(5);
    /// while let Some(tx) = stream.next().await {
    ///    println!("{tx:#?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "pubsub")]
    async fn subscribe_full_pending_transactions(
        &self,
    ) -> TransportResult<alloy_pubsub::Subscription<N::TransactionResponse>> {
        self.root().pubsub_frontend()?;
        let id = self.client().request("eth_subscribe", ("newPendingTransactions", true)).await?;
        self.root().get_subscription(id).await
    }

    /// Subscribe to a stream of logs matching given filter.
    ///
    /// # Errors
    ///
    /// This method is only available on `pubsub` clients, such as WebSockets or IPC, and will
    /// return a [`PubsubUnavailable`](alloy_transport::TransportErrorKind::PubsubUnavailable)
    /// transport error if the client does not support it.
    ///
    /// For a polling alternative available over HTTP, use
    /// [`Provider::watch_logs`]. However, be aware that polling increases
    /// RPC usage drastically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use futures::StreamExt;
    /// use alloy_primitives::keccak256;
    /// use alloy_rpc_types_eth::Filter;
    ///
    /// let signature = keccak256("Transfer(address,address,uint256)".as_bytes());
    ///
    /// let sub = provider.subscribe_logs(&Filter::new().event_signature(signature)).await?;
    /// let mut stream = sub.into_stream().take(5);
    /// while let Some(tx) = stream.next().await {
    ///    println!("{tx:#?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "pubsub")]
    async fn subscribe_logs(
        &self,
        filter: &Filter,
    ) -> TransportResult<alloy_pubsub::Subscription<Log>> {
        self.root().pubsub_frontend()?;
        let id = self.client().request("eth_subscribe", ("logs", filter)).await?;
        self.root().get_subscription(id).await
    }

    /// Subscribe to an RPC event.
    #[cfg(feature = "pubsub")]
    #[auto_impl(keep_default_for(&, &mut, Rc, Arc, Box))]
    async fn subscribe<P, R>(&self, params: P) -> TransportResult<alloy_pubsub::Subscription<R>>
    where
        P: RpcSend,
        R: RpcRecv,
        Self: Sized,
    {
        self.root().pubsub_frontend()?;
        let id = self.client().request("eth_subscribe", params).await?;
        self.root().get_subscription(id).await
    }

    /// Cancels a subscription given the subscription ID.
    #[cfg(feature = "pubsub")]
    async fn unsubscribe(&self, id: B256) -> TransportResult<()> {
        self.root().unsubscribe(id)
    }

    /// Gets syncing info.
    fn syncing(&self) -> ProviderCall<NoParams, SyncStatus> {
        self.client().request_noparams("eth_syncing").into()
    }

    /// Gets the client version.
    #[doc(alias = "web3_client_version")]
    fn get_client_version(&self) -> ProviderCall<NoParams, String> {
        self.client().request_noparams("web3_clientVersion").into()
    }

    /// Gets the `Keccak-256` hash of the given data.
    #[doc(alias = "web3_sha3")]
    fn get_sha3(&self, data: &[u8]) -> ProviderCall<(String,), B256> {
        self.client().request("web3_sha3", (hex::encode_prefixed(data),)).into()
    }

    /// Gets the network ID. Same as `eth_chainId`.
    fn get_net_version(&self) -> ProviderCall<NoParams, U64, u64> {
        self.client()
            .request_noparams("net_version")
            .map_resp(utils::convert_u64 as fn(U64) -> u64)
            .into()
    }

    /* ---------------------------------------- raw calls --------------------------------------- */

    /// Sends a raw JSON-RPC request.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use alloy_rpc_types_eth::BlockNumberOrTag;
    /// use alloy_rpc_client::NoParams;
    ///
    /// // No parameters: `()`
    /// let block_number = provider.raw_request("eth_blockNumber".into(), NoParams::default()).await?;
    ///
    /// // One parameter: `(param,)` or `[param]`
    /// let block = provider.raw_request("eth_getBlockByNumber".into(), (BlockNumberOrTag::Latest,)).await?;
    ///
    /// // Two or more parameters: `(param1, param2, ...)` or `[param1, param2, ...]`
    /// let full_block = provider.raw_request("eth_getBlockByNumber".into(), (BlockNumberOrTag::Latest, true)).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`PubsubUnavailable`]: alloy_transport::TransportErrorKind::PubsubUnavailable
    async fn raw_request<P, R>(&self, method: Cow<'static, str>, params: P) -> TransportResult<R>
    where
        P: RpcSend,
        R: RpcRecv,
        Self: Sized,
    {
        self.client().request(method, &params).await
    }

    /// Sends a raw JSON-RPC request with type-erased parameters and return.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn example(provider: impl alloy_provider::Provider) -> Result<(), Box<dyn std::error::Error>> {
    /// use alloy_rpc_types_eth::BlockNumberOrTag;
    ///
    /// // No parameters: `()`
    /// let params = serde_json::value::to_raw_value(&())?;
    /// let block_number = provider.raw_request_dyn("eth_blockNumber".into(), &params).await?;
    ///
    /// // One parameter: `(param,)` or `[param]`
    /// let params = serde_json::value::to_raw_value(&(BlockNumberOrTag::Latest,))?;
    /// let block = provider.raw_request_dyn("eth_getBlockByNumber".into(), &params).await?;
    ///
    /// // Two or more parameters: `(param1, param2, ...)` or `[param1, param2, ...]`
    /// let params = serde_json::value::to_raw_value(&(BlockNumberOrTag::Latest, true))?;
    /// let full_block = provider.raw_request_dyn("eth_getBlockByNumber".into(), &params).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn raw_request_dyn(
        &self,
        method: Cow<'static, str>,
        params: &RawValue,
    ) -> TransportResult<Box<RawValue>> {
        self.client().request(method, params).await
    }

    /// Creates a new [`TransactionRequest`](alloy_network::Network).
    #[inline]
    fn transaction_request(&self) -> N::TransactionRequest {
        Default::default()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl<N: Network> Provider<N> for RootProvider<N> {
    #[inline]
    fn root(&self) -> &Self {
        self
    }

    #[inline]
    fn client(&self) -> ClientRef<'_> {
        self.inner.client_ref()
    }

    #[inline]
    fn weak_client(&self) -> WeakClient {
        self.inner.weak_client()
    }

    #[inline]
    async fn watch_pending_transaction(
        &self,
        config: PendingTransactionConfig,
    ) -> Result<PendingTransaction, PendingTransactionError> {
        let block_number =
            if let Some(receipt) = self.get_transaction_receipt(*config.tx_hash()).await? {
                // The transaction is already confirmed.
                if config.required_confirmations() <= 1 {
                    return Ok(PendingTransaction::ready(*config.tx_hash()));
                }
                // Transaction has custom confirmations, so let the heart know about its block
                // number and let it handle the situation.
                receipt.block_number()
            } else {
                None
            };

        self.get_heart()
            .watch_tx(config, block_number)
            .await
            .map_err(|_| PendingTransactionError::FailedToRegister)
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Read, str::FromStr, time::Duration};

    use super::*;
    use crate::{builder, ProviderBuilder, WalletProvider};
    use alloy_consensus::Transaction;
    use alloy_network::{AnyNetwork, EthereumWallet, TransactionBuilder};
    use alloy_node_bindings::Anvil;
    use alloy_primitives::{address, b256, bytes, keccak256};
    use alloy_rpc_client::{BuiltInConnectionString, RpcClient};
    use alloy_rpc_types_eth::{request::TransactionRequest, Block};
    use alloy_signer_local::PrivateKeySigner;
    use alloy_transport::layers::{RetryBackoffLayer, RetryPolicy};
    // For layer transport tests
    #[cfg(feature = "hyper")]
    use alloy_transport_http::{
        hyper,
        hyper::body::Bytes as HyperBytes,
        hyper_util::{
            client::legacy::{Client, Error},
            rt::TokioExecutor,
        },
        HyperResponse, HyperResponseFut,
    };
    #[cfg(feature = "hyper")]
    use http_body_util::Full;
    #[cfg(feature = "hyper")]
    use tower::{Layer, Service};

    #[tokio::test]
    async fn test_provider_builder() {
        let provider = RootProvider::builder().with_recommended_fillers().on_anvil();
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num);
    }

    #[tokio::test]
    async fn test_builder_helper_fn() {
        let provider = builder().with_recommended_fillers().on_anvil();
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num);
    }

    #[cfg(feature = "hyper")]
    #[tokio::test]
    async fn test_default_hyper_transport() {
        let anvil = Anvil::new().spawn();
        let hyper_t = alloy_transport_http::HyperTransport::new_hyper(anvil.endpoint_url());

        let rpc_client = alloy_rpc_client::RpcClient::new(hyper_t, true);

        let provider = RootProvider::<Ethereum>::new(rpc_client);
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num);
    }

    #[cfg(feature = "hyper")]
    #[tokio::test]
    async fn test_hyper_layer_transport() {
        struct LoggingLayer;

        impl<S> Layer<S> for LoggingLayer {
            type Service = LoggingService<S>;

            fn layer(&self, inner: S) -> Self::Service {
                LoggingService { inner }
            }
        }

        #[derive(Clone)] // required
        struct LoggingService<S> {
            inner: S,
        }

        impl<S, B> Service<hyper::Request<B>> for LoggingService<S>
        where
            S: Service<hyper::Request<B>, Response = HyperResponse, Error = Error>
                + Clone
                + Send
                + Sync
                + 'static,
            S::Future: Send,
            S::Error: std::error::Error + Send + Sync + 'static,
            B: From<Vec<u8>> + Send + 'static + Clone + Sync + std::fmt::Debug,
        {
            type Response = HyperResponse;
            type Error = Error;
            type Future = HyperResponseFut;

            fn poll_ready(
                &mut self,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), Self::Error>> {
                self.inner.poll_ready(cx)
            }

            fn call(&mut self, req: hyper::Request<B>) -> Self::Future {
                println!("Logging Layer - HyperRequest {req:?}");

                let fut = self.inner.call(req);

                Box::pin(fut)
            }
        }
        use http::header::{self, HeaderValue};
        use tower_http::{
            sensitive_headers::SetSensitiveRequestHeadersLayer, set_header::SetRequestHeaderLayer,
        };
        let anvil = Anvil::new().spawn();
        let hyper_client = Client::builder(TokioExecutor::new()).build_http::<Full<HyperBytes>>();

        // Setup tower serive with multiple layers modifying request headers
        let service = tower::ServiceBuilder::new()
            .layer(SetRequestHeaderLayer::if_not_present(
                header::USER_AGENT,
                HeaderValue::from_static("alloy app"),
            ))
            .layer(SetRequestHeaderLayer::overriding(
                header::AUTHORIZATION,
                HeaderValue::from_static("some-jwt-token"),
            ))
            .layer(SetRequestHeaderLayer::appending(
                header::SET_COOKIE,
                HeaderValue::from_static("cookie-value"),
            ))
            .layer(SetSensitiveRequestHeadersLayer::new([header::AUTHORIZATION])) // Hides the jwt token as sensitive.
            .layer(LoggingLayer)
            .service(hyper_client);

        let layer_transport = alloy_transport_http::HyperClient::with_service(service);

        let http_hyper =
            alloy_transport_http::Http::with_client(layer_transport, anvil.endpoint_url());

        let rpc_client = alloy_rpc_client::RpcClient::new(http_hyper, true);

        let provider = RootProvider::<Ethereum>::new(rpc_client);
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num);

        // Test Cloning with service
        let cloned_t = provider.client().transport().clone();

        let rpc_client = alloy_rpc_client::RpcClient::new(cloned_t, true);

        let provider = RootProvider::<Ethereum>::new(rpc_client);
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num);
    }

    #[cfg(feature = "hyper")]
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn test_auth_layer_transport() {
        crate::ext::test::async_ci_only(|| async move {
            use alloy_node_bindings::Reth;
            use alloy_rpc_types_engine::JwtSecret;
            use alloy_transport_http::{AuthLayer, AuthService, Http, HyperClient};

            let secret = JwtSecret::random();

            let reth =
                Reth::new().arg("--rpc.jwtsecret").arg(hex::encode(secret.as_bytes())).spawn();

            let hyper_client =
                Client::builder(TokioExecutor::new()).build_http::<Full<HyperBytes>>();

            let service =
                tower::ServiceBuilder::new().layer(AuthLayer::new(secret)).service(hyper_client);

            let layer_transport: HyperClient<
                Full<HyperBytes>,
                AuthService<
                    Client<
                        alloy_transport_http::hyper_util::client::legacy::connect::HttpConnector,
                        Full<HyperBytes>,
                    >,
                >,
            > = HyperClient::with_service(service);

            let http_hyper = Http::with_client(layer_transport, reth.endpoint_url());

            let rpc_client = alloy_rpc_client::RpcClient::new(http_hyper, true);

            let provider = RootProvider::<Ethereum>::new(rpc_client);

            let num = provider.get_block_number().await.unwrap();
            assert_eq!(0, num);
        })
        .await;
    }

    #[tokio::test]
    async fn test_builder_helper_fn_any_network() {
        let anvil = Anvil::new().spawn();
        let provider =
            builder::<AnyNetwork>().with_recommended_fillers().on_http(anvil.endpoint_url());
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num);
    }

    #[cfg(feature = "reqwest")]
    #[tokio::test]
    async fn object_safety() {
        let provider = ProviderBuilder::new().on_anvil();

        let refdyn = &provider as &dyn Provider<_>;
        let num = refdyn.get_block_number().await.unwrap();
        assert_eq!(0, num);
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn subscribe_blocks_http() {
        let provider = ProviderBuilder::new().on_anvil_with_config(|a| a.block_time(1));

        let err = provider.subscribe_blocks().await.unwrap_err();
        let alloy_json_rpc::RpcError::Transport(
            alloy_transport::TransportErrorKind::PubsubUnavailable,
        ) = err
        else {
            panic!("{err:?}");
        };
    }

    // Ensures we can connect to a websocket using `wss`.
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn websocket_tls_setup() {
        for url in [
            "wss://eth-mainnet.ws.alchemyapi.io/v2/MdZcimFJ2yz2z6pw21UYL-KNA0zmgX-F",
            "wss://mainnet.infura.io/ws/v3/b0f825787ba840af81e46c6a64d20754",
        ] {
            let _ = ProviderBuilder::<_, _, Ethereum>::default().on_builtin(url).await.unwrap();
        }
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn subscribe_blocks_ws() {
        use futures::stream::StreamExt;

        let anvil = Anvil::new().block_time(1).spawn();
        let ws = alloy_rpc_client::WsConnect::new(anvil.ws_endpoint());
        let client = alloy_rpc_client::RpcClient::connect_pubsub(ws).await.unwrap();
        let provider = RootProvider::<Ethereum>::new(client);

        let sub = provider.subscribe_blocks().await.unwrap();
        let mut stream = sub.into_stream().take(2);
        let mut n = 1;
        while let Some(header) = stream.next().await {
            assert_eq!(header.number, n);
            n += 1;
        }
    }

    #[tokio::test]
    #[cfg(feature = "ws")]
    async fn subscribe_blocks_ws_remote() {
        use futures::stream::StreamExt;

        let url = "wss://eth-mainnet.g.alchemy.com/v2/viFmeVzhg6bWKVMIWWS8MhmzREB-D4f7";
        let ws = alloy_rpc_client::WsConnect::new(url);
        let Ok(client) = alloy_rpc_client::RpcClient::connect_pubsub(ws).await else { return };
        let provider = RootProvider::<Ethereum>::new(client);
        let sub = provider.subscribe_blocks().await.unwrap();
        let mut stream = sub.into_stream().take(1);
        while let Some(header) = stream.next().await {
            println!("New block {:?}", header);
            assert!(header.number > 0);
        }
    }

    #[tokio::test]
    async fn test_custom_retry_policy() {
        #[derive(Debug, Clone)]
        struct CustomPolicy;
        impl RetryPolicy for CustomPolicy {
            fn should_retry(&self, _err: &alloy_transport::TransportError) -> bool {
                true
            }

            fn backoff_hint(
                &self,
                _error: &alloy_transport::TransportError,
            ) -> Option<std::time::Duration> {
                None
            }
        }

        let retry_layer = RetryBackoffLayer::new_with_policy(10, 100, 10000, CustomPolicy);
        let anvil = Anvil::new().spawn();
        let client = RpcClient::builder().layer(retry_layer).http(anvil.endpoint_url());

        let provider = RootProvider::<Ethereum>::new(client);
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num);
    }

    #[tokio::test]
    async fn test_send_tx() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();
        let tx = TransactionRequest {
            value: Some(U256::from(100)),
            to: Some(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into()),
            gas_price: Some(20e9 as u128),
            gas: Some(21000),
            ..Default::default()
        };

        let builder = provider.send_transaction(tx.clone()).await.expect("failed to send tx");
        let hash1 = *builder.tx_hash();
        let hash2 = builder.watch().await.expect("failed to await pending tx");
        assert_eq!(hash1, hash2);

        let builder = provider.send_transaction(tx).await.expect("failed to send tx");
        let hash1 = *builder.tx_hash();
        let hash2 =
            builder.get_receipt().await.expect("failed to await pending tx").transaction_hash;
        assert_eq!(hash1, hash2);
    }

    #[tokio::test]
    async fn test_watch_confirmed_tx() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();
        let tx = TransactionRequest {
            value: Some(U256::from(100)),
            to: Some(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into()),
            gas_price: Some(20e9 as u128),
            gas: Some(21000),
            ..Default::default()
        };

        let builder = provider.send_transaction(tx.clone()).await.expect("failed to send tx");
        let hash1 = *builder.tx_hash();

        // Wait until tx is confirmed.
        loop {
            if provider
                .get_transaction_receipt(hash1)
                .await
                .expect("failed to await pending tx")
                .is_some()
            {
                break;
            }
        }

        // Submit another tx.
        let tx2 = TransactionRequest {
            value: Some(U256::from(100)),
            to: Some(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into()),
            gas_price: Some(20e9 as u128),
            gas: Some(21000),
            ..Default::default()
        };
        provider.send_transaction(tx2).await.expect("failed to send tx").watch().await.unwrap();

        // Only subscribe for watching _after_ tx was confirmed and we submitted a new one.
        let watch = builder.watch();
        // Wrap watch future in timeout to prevent it from hanging.
        let watch_with_timeout = tokio::time::timeout(Duration::from_secs(1), watch);
        let hash2 = watch_with_timeout
            .await
            .expect("Watching tx timed out")
            .expect("failed to await pending tx");
        assert_eq!(hash1, hash2);
    }

    #[tokio::test]
    async fn gets_block_number() {
        let provider = ProviderBuilder::new().on_anvil();
        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num)
    }

    #[tokio::test]
    async fn gets_block_number_with_raw_req() {
        let provider = ProviderBuilder::new().on_anvil();
        let num: U64 =
            provider.raw_request("eth_blockNumber".into(), NoParams::default()).await.unwrap();
        assert_eq!(0, num.to::<u64>())
    }

    #[cfg(feature = "anvil-api")]
    #[tokio::test]
    async fn gets_transaction_count() {
        let provider = ProviderBuilder::new().on_anvil();
        let accounts = provider.get_accounts().await.unwrap();
        let sender = accounts[0];

        // Initial tx count should be 0
        let count = provider.get_transaction_count(sender).await.unwrap();
        assert_eq!(count, 0);

        // Send Tx
        let tx = TransactionRequest {
            value: Some(U256::from(100)),
            from: Some(sender),
            to: Some(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into()),
            gas_price: Some(20e9 as u128),
            gas: Some(21000),
            ..Default::default()
        };
        let _ = provider.send_transaction(tx).await.unwrap().get_receipt().await;

        // Tx count should be 1
        let count = provider.get_transaction_count(sender).await.unwrap();
        assert_eq!(count, 1);

        // Tx count should be 0 at block 0
        let count = provider.get_transaction_count(sender).block_id(0.into()).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn gets_block_by_hash() {
        let provider = ProviderBuilder::new().on_anvil();
        let num = 0;
        let tag: BlockNumberOrTag = num.into();
        let block =
            provider.get_block_by_number(tag, BlockTransactionsKind::Full).await.unwrap().unwrap();
        let hash = block.header.hash;
        let block =
            provider.get_block_by_hash(hash, BlockTransactionsKind::Full).await.unwrap().unwrap();
        assert_eq!(block.header.hash, hash);
    }

    #[tokio::test]
    async fn gets_block_by_hash_with_raw_req() {
        let provider = ProviderBuilder::new().on_anvil();
        let num = 0;
        let tag: BlockNumberOrTag = num.into();
        let block =
            provider.get_block_by_number(tag, BlockTransactionsKind::Full).await.unwrap().unwrap();
        let hash = block.header.hash;
        let block: Block = provider
            .raw_request::<(B256, bool), Block>("eth_getBlockByHash".into(), (hash, true))
            .await
            .unwrap();
        assert_eq!(block.header.hash, hash);
    }

    #[tokio::test]
    async fn gets_block_by_number_full() {
        let provider = ProviderBuilder::new().on_anvil();
        let num = 0;
        let tag: BlockNumberOrTag = num.into();
        let block =
            provider.get_block_by_number(tag, BlockTransactionsKind::Full).await.unwrap().unwrap();
        assert_eq!(block.header.number, num);
    }

    #[tokio::test]
    async fn gets_block_by_number() {
        let provider = ProviderBuilder::new().on_anvil();
        let num = 0;
        let tag: BlockNumberOrTag = num.into();
        let block =
            provider.get_block_by_number(tag, BlockTransactionsKind::Full).await.unwrap().unwrap();
        assert_eq!(block.header.number, num);
    }

    #[tokio::test]
    async fn gets_client_version() {
        let provider = ProviderBuilder::new().on_anvil();
        let version = provider.get_client_version().await.unwrap();
        assert!(version.contains("anvil"), "{version}");
    }

    #[tokio::test]
    async fn gets_sha3() {
        let provider = ProviderBuilder::new().on_anvil();
        let data = b"alloy";
        let hash = provider.get_sha3(data).await.unwrap();
        assert_eq!(hash, keccak256(data));
    }

    #[tokio::test]
    async fn gets_chain_id() {
        let dev_chain_id: u64 = 13371337;

        let provider = ProviderBuilder::new().on_anvil_with_config(|a| a.chain_id(dev_chain_id));

        let chain_id = provider.get_chain_id().await.unwrap();
        assert_eq!(chain_id, dev_chain_id);
    }

    #[tokio::test]
    async fn gets_network_id() {
        let dev_chain_id: u64 = 13371337;
        let provider = ProviderBuilder::new().on_anvil_with_config(|a| a.chain_id(dev_chain_id));

        let chain_id = provider.get_net_version().await.unwrap();
        assert_eq!(chain_id, dev_chain_id);
    }

    #[tokio::test]
    async fn gets_storage_at() {
        let provider = ProviderBuilder::new().on_anvil();
        let addr = Address::with_last_byte(16);
        let storage = provider.get_storage_at(addr, U256::ZERO).await.unwrap();
        assert_eq!(storage, U256::ZERO);
    }

    #[tokio::test]
    async fn gets_transaction_by_hash_not_found() {
        let provider = ProviderBuilder::new().on_anvil();
        let tx_hash = b256!("5c03fab9114ceb98994b43892ade87ddfd9ae7e8f293935c3bd29d435dc9fd95");
        let tx = provider.get_transaction_by_hash(tx_hash).await.expect("failed to fetch tx");

        assert!(tx.is_none());
    }

    #[tokio::test]
    async fn gets_transaction_by_hash() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        let req = TransactionRequest::default()
            .from(provider.default_signer_address())
            .to(Address::repeat_byte(5))
            .value(U256::ZERO)
            .input(bytes!("deadbeef").into());

        let tx_hash = *provider.send_transaction(req).await.expect("failed to send tx").tx_hash();

        let tx = provider
            .get_transaction_by_hash(tx_hash)
            .await
            .expect("failed to fetch tx")
            .expect("tx not included");
        assert_eq!(tx.input(), &bytes!("deadbeef"));
    }

    #[tokio::test]
    #[ignore]
    async fn gets_logs() {
        let provider = ProviderBuilder::new().on_anvil();
        let filter = Filter::new()
            .at_block_hash(b256!(
                "b20e6f35d4b46b3c4cd72152faec7143da851a0dc281d390bdd50f58bfbdb5d3"
            ))
            .event_signature(b256!(
                "e1fffcc4923d04b559f4d29a8bfc6cda04eb5b0d3c460751c2402c5c5cc9109c"
            ));
        let logs = provider.get_logs(&filter).await.unwrap();
        assert_eq!(logs.len(), 1);
    }

    #[tokio::test]
    #[ignore]
    async fn gets_tx_receipt() {
        let provider = ProviderBuilder::new().on_anvil();
        let receipt = provider
            .get_transaction_receipt(b256!(
                "5c03fab9114ceb98994b43892ade87ddfd9ae7e8f293935c3bd29d435dc9fd95"
            ))
            .await
            .unwrap();
        assert!(receipt.is_some());
        let receipt = receipt.unwrap();
        assert_eq!(
            receipt.transaction_hash,
            b256!("5c03fab9114ceb98994b43892ade87ddfd9ae7e8f293935c3bd29d435dc9fd95")
        );
    }

    #[tokio::test]
    async fn gets_max_priority_fee_per_gas() {
        let provider = ProviderBuilder::new().on_anvil();
        let _fee = provider.get_max_priority_fee_per_gas().await.unwrap();
    }

    #[tokio::test]
    async fn gets_fee_history() {
        let provider = ProviderBuilder::new().on_anvil();
        let block_number = provider.get_block_number().await.unwrap();
        let fee_history = provider
            .get_fee_history(
                utils::EIP1559_FEE_ESTIMATION_PAST_BLOCKS,
                BlockNumberOrTag::Number(block_number),
                &[utils::EIP1559_FEE_ESTIMATION_REWARD_PERCENTILE],
            )
            .await
            .unwrap();
        assert_eq!(fee_history.oldest_block, 0_u64);
    }

    #[tokio::test]
    async fn gets_block_transaction_count_by_hash() {
        let provider = ProviderBuilder::new().on_anvil();
        let block = provider
            .get_block(BlockId::latest(), BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();
        let hash = block.header.hash;
        let tx_count = provider.get_block_transaction_count_by_hash(hash).await.unwrap();
        assert!(tx_count.is_some());
    }

    #[tokio::test]
    async fn gets_block_transaction_count_by_number() {
        let provider = ProviderBuilder::new().on_anvil();
        let tx_count =
            provider.get_block_transaction_count_by_number(BlockNumberOrTag::Latest).await.unwrap();
        assert!(tx_count.is_some());
    }

    #[tokio::test]
    async fn gets_block_receipts() {
        let provider = ProviderBuilder::new().on_anvil();
        let receipts =
            provider.get_block_receipts(BlockId::Number(BlockNumberOrTag::Latest)).await.unwrap();
        assert!(receipts.is_some());
    }

    #[tokio::test]
    async fn sends_raw_transaction() {
        let provider = ProviderBuilder::new().on_anvil();
        let pending = provider
            .send_raw_transaction(
                // Transfer 1 ETH from default EOA address to the Genesis address.
                bytes!("f865808477359400825208940000000000000000000000000000000000000000018082f4f5a00505e227c1c636c76fac55795db1a40a4d24840d81b40d2fe0cc85767f6bd202a01e91b437099a8a90234ac5af3cb7ca4fb1432e133f75f9a91678eaf5f487c74b").as_ref()
            )
            .await.unwrap();
        assert_eq!(
            pending.tx_hash().to_string(),
            "0x9dae5cf33694a02e8a7d5de3fe31e9d05ca0ba6e9180efac4ab20a06c9e598a3"
        );
    }

    #[tokio::test]
    async fn connect_boxed() {
        let anvil = Anvil::new().spawn();

        let provider = RootProvider::<Ethereum>::connect(anvil.endpoint().as_str()).await;

        match provider {
            Ok(provider) => {
                let num = provider.get_block_number().await.unwrap();
                assert_eq!(0, num);
            }
            Err(e) => {
                assert_eq!(
                    format!("{}",e),
                    "hyper not supported by BuiltinConnectionString. Please instantiate a hyper client manually"
                );
            }
        }
    }

    #[tokio::test]
    async fn any_network_wallet_filler() {
        use alloy_serde::WithOtherFields;
        let anvil = Anvil::new().spawn();
        let signer: PrivateKeySigner =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse().unwrap();
        let wallet = EthereumWallet::from(signer);

        let provider = ProviderBuilder::new()
            .network::<AnyNetwork>()
            .wallet(wallet)
            .on_http(anvil.endpoint_url());

        let tx = TransactionRequest::default()
            .with_to(address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"))
            .value(U256::from(325235));

        let tx = WithOtherFields::new(tx);

        let builder = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

        assert!(builder.status());
    }

    #[tokio::test]
    async fn builtin_connect_boxed() {
        let anvil = Anvil::new().spawn();

        let conn: BuiltInConnectionString = anvil.endpoint().parse().unwrap();

        let transport = conn.connect_boxed().await.unwrap();

        let client = alloy_rpc_client::RpcClient::new(transport, true);

        let provider = RootProvider::<Ethereum>::new(client);

        let num = provider.get_block_number().await.unwrap();
        assert_eq!(0, num);
    }

    #[tokio::test]
    async fn test_uncle_count() {
        let provider = ProviderBuilder::new().on_anvil();

        let count = provider.get_uncle_count(0.into()).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    #[cfg(any(
        feature = "reqwest-default-tls",
        feature = "reqwest-rustls-tls",
        feature = "reqwest-native-tls",
    ))]
    async fn call_mainnet() {
        use alloy_network::TransactionBuilder;
        use alloy_sol_types::SolValue;

        let url = "https://eth-mainnet.alchemyapi.io/v2/jGiK5vwDfC3F4r0bqukm-W2GqgdrxdSr";
        let provider = ProviderBuilder::new().on_http(url.parse().unwrap());
        let req = TransactionRequest::default()
            .with_to(address!("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2")) // WETH
            .with_input(bytes!("06fdde03")); // `name()`
        let result = provider.call(&req).await.unwrap();
        assert_eq!(String::abi_decode(&result, true).unwrap(), "Wrapped Ether");

        let result = provider.call(&req).block(0.into()).await.unwrap();
        assert_eq!(result.to_string(), "0x");
    }

    #[tokio::test]
    async fn call_many_mainnet() {
        use alloy_rpc_types_eth::{BlockOverrides, StateContext};

        let url = "https://docs-demo.quiknode.pro/";
        let provider = ProviderBuilder::new().on_http(url.parse().unwrap());
        let tx1 = TransactionRequest::default()
            .with_to(address!("6b175474e89094c44da98b954eedeac495271d0f"))
            .with_gas_limit(1000000)
            .with_gas_price(2023155498)
            .with_input(hex!("a9059cbb000000000000000000000000bc0E63965946815d105E7591407704e6e1964E590000000000000000000000000000000000000000000000000000000005f5e100"));
        let tx2 = TransactionRequest::default()
            .with_to(address!("833589fcd6edb6e08f4c7c32d4f71b54bda02913"))
            .with_gas_price(2023155498)
            .with_input(hex!(
                "70a08231000000000000000000000000bc0E63965946815d105E7591407704e6e1964E59"
            ));

        let transactions = vec![tx1.clone(), tx2.clone()];

        let block_override =
            BlockOverrides { number: Some(U256::from(12279785)), ..Default::default() };

        let bundles = vec![Bundle { transactions, block_override: Some(block_override.clone()) }];

        let context = StateContext {
            block_number: Some(BlockId::number(12279785)),
            transaction_index: Some(1.into()),
        };

        let results = provider.call_many(&bundles).context(&context).await.unwrap();

        let tx1_res = EthCallResponse {
            value: Some(
                hex!("0000000000000000000000000000000000000000000000000000000000000001").into(),
            ),
            error: None,
        };
        let tx2_res = EthCallResponse { value: Some(Bytes::new()), error: None };
        let expected = vec![vec![tx1_res.clone(), tx2_res.clone()]];

        assert_eq!(results, expected);

        // Two bundles
        let bundles = vec![
            Bundle {
                transactions: vec![tx1.clone()],
                block_override: Some(block_override.clone()),
            },
            Bundle {
                transactions: vec![tx2.clone()],
                block_override: Some(block_override.clone()),
            },
        ];

        let results = provider.call_many(&bundles).context(&context).await.unwrap();
        let expected = vec![vec![tx1_res.clone()], vec![tx2_res.clone()]];
        assert_eq!(results, expected);

        // Two bundles by extending existing.
        let b1 =
            vec![Bundle { transactions: vec![tx1], block_override: Some(block_override.clone()) }];
        let b2 = vec![Bundle { transactions: vec![tx2], block_override: Some(block_override) }];

        let results = provider.call_many(&b1).context(&context).extend_bundles(&b2).await.unwrap();
        assert_eq!(results, expected);
    }

    #[tokio::test]
    #[cfg(feature = "hyper-tls")]
    async fn hyper_https() {
        let url = "https://eth-mainnet.alchemyapi.io/v2/jGiK5vwDfC3F4r0bqukm-W2GqgdrxdSr";

        // With the `hyper` feature enabled .on_builtin builds the provider based on
        // `HyperTransport`.
        let provider = ProviderBuilder::new().on_builtin(url).await.unwrap();

        let _num = provider.get_block_number().await.unwrap();
    }

    #[tokio::test]
    async fn test_empty_transactions() {
        let provider = ProviderBuilder::new().on_anvil();

        let block = provider
            .get_block_by_number(0.into(), BlockTransactionsKind::Hashes)
            .await
            .unwrap()
            .unwrap();
        assert!(block.transactions.is_hashes());
    }

    #[tokio::test]
    async fn disable_test() {
        let provider = ProviderBuilder::new()
            .disable_recommended_fillers()
            .with_cached_nonce_management()
            .on_anvil();

        let tx = TransactionRequest::default()
            .with_kind(alloy_primitives::TxKind::Create)
            .value(U256::from(1235))
            .with_input(Bytes::from_str("ffffffffffffff").unwrap());

        let err = provider.send_transaction(tx).await.unwrap_err().to_string();
        assert!(err.contains("missing properties: [(\"NonceManager\", [\"from\"])]"));
    }

    #[tokio::test]
    async fn capture_anvil_logs() {
        let mut anvil = Anvil::new().keep_stdout().spawn();

        let provider = ProviderBuilder::new().on_http(anvil.endpoint_url());

        let tx = TransactionRequest::default()
            .with_from(address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"))
            .with_to(address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"))
            .value(U256::from(100));

        let _ = provider.send_transaction(tx).await.unwrap().get_receipt().await.unwrap();

        anvil.child_mut().kill().unwrap();

        let mut output = String::new();
        anvil.child_mut().stdout.take().unwrap().read_to_string(&mut output).unwrap();

        assert_eq!(anvil.chain_id(), 31337);
        assert_eq!(anvil.addresses().len(), 10);
        assert_eq!(anvil.keys().len(), 10);

        assert!(output.contains("eth_sendTransaction"));
        assert!(output.contains("Block Number: 1"))
    }
}
