//! A Multicall Builder

use crate::Provider;
use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{address, Address, BlockNumber, Bytes, B256, U256};
use alloy_rpc_types_eth::{state::StateOverride, BlockId};
use alloy_sol_types::SolCall;
use bindings::IMulticall3::{
    blockAndAggregateCall, blockAndAggregateReturn, tryBlockAndAggregateCall,
    tryBlockAndAggregateReturn, Call, Call3, Call3Value,
};

/// Multicall bindings
pub mod bindings;
use crate::provider::multicall::bindings::IMulticall3::{
    aggregate3Call, aggregate3ValueCall, aggregateCall, getBasefeeCall, getBlockHashCall,
    getBlockNumberCall, getChainIdCall, getCurrentBlockCoinbaseCall, getCurrentBlockDifficultyCall,
    getCurrentBlockGasLimitCall, getCurrentBlockTimestampCall, getEthBalanceCall,
    getLastBlockHashCall, tryAggregateCall, tryAggregateReturn,
};

mod inner_types;
pub use inner_types::{
    CallInfoTrait, CallItem, CallItemBuilder, Dynamic, Failure, MulticallError, MulticallItem,
    Result,
};

mod tuple;
use tuple::TuplePush;
pub use tuple::{CallTuple, Empty};

/// Default address for the Multicall3 contract on most chains. See: <https://github.com/mds1/multicall>
pub const MULTICALL3_ADDRESS: Address = address!("cA11bde05977b3631167028862bE2a173976CA11");

/// A Multicall3 builder
///
/// This builder implements a simple API interface to build and execute multicalls using the
/// [`IMultiCall3`](crate::bindings::IMulticall3) contract which is available on 270+
/// chains.
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
///     let (total_supply, balance) = multicall.aggregate().await.unwrap();
///
///     println!("Total Supply: {:?}, Balance: {:?}", total_supply, balance);
/// }
/// ```
#[derive(Debug)]
pub struct MulticallBuilder<T: CallTuple, P: Provider<N>, N: Network> {
    /// Batched calls
    calls: Vec<Call3Value>,
    /// The provider to use
    provider: P,
    /// The [`BlockId`] to use for the call
    block: Option<BlockId>,
    /// The [`StateOverride`] for the call
    state_override: Option<StateOverride>,
    /// This is the address of the [`IMulticall3`](crate::bindings::IMulticall3)
    /// contract.
    ///
    /// By default it is set to [`MULTICALL3_ADDRESS`].
    address: Address,
    _pd: std::marker::PhantomData<(T, N)>,
}

impl<P, N> MulticallBuilder<Empty, P, N>
where
    P: Provider<N>,
    N: Network,
{
    /// Instantiate a new [`MulticallBuilder`]
    pub fn new(provider: P) -> Self {
        Self {
            calls: Vec::new(),
            provider,
            _pd: Default::default(),
            block: None,
            state_override: None,
            address: MULTICALL3_ADDRESS,
        }
    }
}

impl<D: SolCall + 'static, P, N> MulticallBuilder<Dynamic<D>, P, N>
where
    P: Provider<N>,
    N: Network,
{
    /// Instantiate a new [`MulticallBuilder`] that restricts the calls to a specific call type.
    ///
    /// Multicalls made using this builder return a vector of the decoded return values.
    ///
    /// An example would be trying to fetch multiple ERC20 balances of an address.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// use alloy_primitives::address;
    /// use alloy_provider::{MulticallBuilder, Provider, ProviderBuilder};
    /// use alloy_sol_types::sol;
    ///
    /// sol! {
    ///   #[sol(rpc)]
    ///   #[derive(Debug, PartialEq)]
    ///   interface ERC20 {
    ///     function balanceOf(address owner) external view returns (uint256 balance);
    ///   }
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///    let weth = address!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2");
    ///    let usdc = address!("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    ///     
    ///    let provider = ProviderBuilder::new().on_http("https://eth.merkle.io".parse().unwrap());
    ///    let weth = ERC20::new(weth, &provider);
    ///    let usdc = ERC20::new(usdc, &provider);
    ///
    ///    let owner = address!("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045");
    ///
    ///    let mut erc20_balances = MulticallBuilder::new_dynamic(provider);
    ///
    ///    for token in &[weth, usdc] {
    ///        erc20_balances = erc20_balances.add_dynamic(token.balanceOf(owner));
    ///    }
    ///
    ///    let balances: Vec<ERC20::balanceOfReturn> = erc20_balances.aggregate().await.unwrap();
    ///
    ///    let weth_bal = &balances[0];
    ///    let usdc_bal = &balances[1];
    ///    println!("WETH Balance: {:?}, USDC Balance: {:?}", weth_bal, usdc_bal);
    /// }
    pub fn new_dynamic(provider: P) -> Self {
        Self {
            calls: Vec::new(),
            provider,
            block: None,
            state_override: None,
            address: MULTICALL3_ADDRESS,
            _pd: Default::default(),
        }
    }

    /// Add a dynamic call to the builder
    pub fn add_dynamic(mut self, item: impl MulticallItem<Decoder = D>) -> Self {
        let target = item.target();
        let input = item.input();

        let call = CallItem::<D>::new(target, input);

        self.calls.push(call.to_call3_value());
        self
    }

    /// Extend the builder with a sequence of calls
    pub fn extend(
        mut self,
        items: impl IntoIterator<Item = impl MulticallItem<Decoder = D>>,
    ) -> Self {
        for item in items {
            self = self.add_dynamic(item);
        }
        self
    }
}

impl<T, P, N> MulticallBuilder<T, &P, N>
where
    T: CallTuple,
    P: Provider<N> + Clone,
    N: Network,
{
    /// Clones the underlying provider and returns a new [`MulticallBuilder`].
    pub fn with_cloned_provider(&self) -> MulticallBuilder<Empty, P, N> {
        MulticallBuilder {
            calls: Vec::new(),
            provider: self.provider.clone(),
            block: None,
            state_override: None,
            address: MULTICALL3_ADDRESS,
            _pd: Default::default(),
        }
    }
}

impl<T, P, N> MulticallBuilder<T, P, N>
where
    T: CallTuple,
    P: Provider<N>,
    N: Network,
{
    /// Set the address of the multicall3 contract
    ///
    /// Default is [`MULTICALL3_ADDRESS`].
    pub fn address(mut self, address: Address) -> Self {
        self.address = address;
        self
    }

    /// Sets the block to be used for the call.
    pub fn block(mut self, block: BlockId) -> Self {
        self.block = Some(block);
        self
    }

    /// Set the state overrides for the call.
    pub fn overrides(mut self, state_override: StateOverride) -> Self {
        self.state_override = Some(state_override);
        self
    }

    /// Appends a [`SolCall`] to the stack.
    #[allow(clippy::should_implement_trait)]
    pub fn add<Item: MulticallItem>(self, item: Item) -> MulticallBuilder<T::Pushed, P, N>
    where
        Item::Decoder: 'static,
        T: TuplePush<Item::Decoder>,
        <T as TuplePush<Item::Decoder>>::Pushed: CallTuple,
    {
        let target = item.target();
        let input = item.input();

        let call = CallItem::<Item::Decoder>::new(target, input);

        self.add_call(call)
    }

    /// Appends a [`CallItem`] to the stack.
    pub fn add_call<D>(mut self, call: CallItem<D>) -> MulticallBuilder<T::Pushed, P, N>
    where
        D: SolCall + 'static,
        T: TuplePush<D>,
        <T as TuplePush<D>>::Pushed: CallTuple,
    {
        self.calls.push(call.to_call3_value());
        MulticallBuilder {
            calls: self.calls,
            provider: self.provider,
            block: self.block,
            state_override: self.state_override,
            address: self.address,
            _pd: Default::default(),
        }
    }

    /// Calls the `aggregate` function
    ///
    /// Requires that all calls succeed, else reverts.
    ///
    /// ## Solidity Function Signature
    ///
    /// ```ignore
    /// sol! {
    ///     function aggregate(Call[] memory calls) external returns (uint256 blockNumber, bytes[] memory returnData);
    /// }
    /// ```
    ///
    /// ## Returns
    ///
    /// - `returnData`: A tuple of the decoded return values for the calls
    ///
    /// One can obtain the block context such as block number and block hash by using the
    /// [MulticallBuilder::block_and_aggregate] function.
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
    ///     let (total_supply, balance) = multicall.aggregate().await.unwrap();
    ///
    ///     println!("Total Supply: {:?}, Balance: {:?}", total_supply, balance);
    /// }
    /// ```
    pub async fn aggregate(&self) -> Result<T::SuccessReturns> {
        let calls = self
            .calls
            .iter()
            .map(|c| Call { target: c.target, callData: c.callData.clone() })
            .collect::<Vec<_>>();
        let call = aggregateCall { calls: calls.to_vec() };
        let output = self.build_and_call(call, None).await?;
        T::decode_returns(&output.returnData)
    }

    /// Call the `tryAggregate` function
    ///
    /// Allows for calls to fail by setting `require_success` to false.
    ///
    /// ## Solidity Function Signature
    ///
    /// ```ignore
    /// sol! {
    ///     function tryAggregate(bool requireSuccess, Call[] calldata calls) external payable returns (Result[] memory returnData);
    /// }
    /// ```
    ///
    /// ## Returns
    ///
    /// - A tuple of the decoded return values for the calls.
    /// - Each return value is wrapped in a [`Result`] struct.
    /// - The [`Result::Ok`] variant contains the decoded return value.
    /// - The [`Result::Err`] variant contains the [`Failure`] struct which holds the
    ///   index(-position) of the call and the returned data as [`Bytes`].
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
    ///     let (total_supply, balance) = multicall.try_aggregate(true).await.unwrap();
    ///
    ///     assert!(total_supply.is_ok());
    ///     assert!(balance.is_ok());
    /// }
    /// ```
    pub async fn try_aggregate(&self, require_success: bool) -> Result<T::Returns> {
        let calls = &self
            .calls
            .iter()
            .map(|c| Call { target: c.target, callData: c.callData.clone() })
            .collect::<Vec<_>>();
        let call = tryAggregateCall { requireSuccess: require_success, calls: calls.to_vec() };
        let output = self.build_and_call(call, None).await?;
        let tryAggregateReturn { returnData } = output;
        T::decode_return_results(&returnData)
    }

    /// Call the `aggregate3` function
    ///
    /// Doesn't require that all calls succeed, reverts only if a call with `allowFailure` set to
    /// false, fails.
    ///
    /// By default, adding a call via [`MulticallBuilder::add`] sets `allow_failure` to false.
    ///
    /// You can add a call that allows failure by using [`MulticallBuilder::add_call`], and setting
    /// `allow_failure` to true in [`CallItem`].
    ///
    /// ## Solidity Function Signature
    ///
    /// ```ignore
    /// sol! {
    ///     function aggregate3(Call3[] calldata calls) external payable returns (Result[] memory returnData);
    /// }
    /// ```
    ///
    /// ## Returns
    ///
    /// - A tuple of the decoded return values for the calls.
    /// - Each return value is wrapped in a [`Result`] struct.
    /// - The [`Result::Ok`] variant contains the decoded return value.
    /// - The [`Result::Err`] variant contains the [`Failure`] struct which holds the
    ///   index(-position) of the call and the returned data as [`Bytes`].
    pub async fn aggregate3(&self) -> Result<T::Returns> {
        let calls = self
            .calls
            .iter()
            .map(|c| Call3 {
                target: c.target,
                callData: c.callData.clone(),
                allowFailure: c.allowFailure,
            })
            .collect::<Vec<_>>();
        let call = aggregate3Call { calls: calls.to_vec() };
        let output = self.build_and_call(call, None).await?;
        T::decode_return_results(&output.returnData)
    }

    /// Call the `aggregate3Value` function
    ///
    /// Similar to `aggregate3` allows for calls to fail. Moreover, it allows for calling into
    /// `payable` functions with the `value` parameter.
    ///
    /// One can set the `value` field in the [`CallItem`] struct and use
    /// [`MulticallBuilder::add_call`] to add it to the stack.
    ///
    /// It is important to note the `aggregate3Value` only succeeds when `msg.value` is _strictly_
    /// equal to the sum of the values of all calls. Summing up the values of all calls and setting
    /// it in the transaction request is handled internally by the builder.
    ///
    /// ## Solidity Function Signature
    ///
    /// ```ignore
    /// sol! {
    ///    function aggregate3Value(Call3Value[] calldata calls) external payable returns (Result[] memory returnData);
    /// }
    /// ```
    ///
    /// ## Returns
    ///
    /// - A tuple of the decoded return values for the calls.
    /// - Each return value is wrapped in a [`Result`] struct.
    /// - The [`Result::Ok`] variant contains the decoded return value.
    /// - The [`Result::Err`] variant contains the [`Failure`] struct which holds the
    ///   index(-position) of the call and the returned data as [`Bytes`].
    pub async fn aggregate3_value(&self) -> Result<T::Returns> {
        let total_value = self.calls.iter().map(|c| c.value).fold(U256::ZERO, |acc, x| acc + x);
        let call = aggregate3ValueCall { calls: self.calls.to_vec() };
        let output = self.build_and_call(call, Some(total_value)).await?;
        T::decode_return_results(&output.returnData)
    }

    /// Call the `blockAndAggregate` function
    pub async fn block_and_aggregate(&self) -> Result<(u64, B256, T::SuccessReturns)> {
        let calls = self
            .calls
            .iter()
            .map(|c| Call { target: c.target, callData: c.callData.clone() })
            .collect::<Vec<_>>();
        let call = blockAndAggregateCall { calls: calls.to_vec() };
        let output = self.build_and_call(call, None).await?;
        let blockAndAggregateReturn { blockNumber, blockHash, returnData } = output;
        let result = T::decode_return_results(&returnData)?;
        Ok((blockNumber.to::<u64>(), blockHash, T::try_into_success(result)?))
    }

    /// Call the `tryBlockAndAggregate` function
    pub async fn try_block_and_aggregate(
        &self,
        require_success: bool,
    ) -> Result<(u64, B256, T::Returns)> {
        let calls = self
            .calls
            .iter()
            .map(|c| Call { target: c.target, callData: c.callData.clone() })
            .collect::<Vec<_>>();
        let call =
            tryBlockAndAggregateCall { requireSuccess: require_success, calls: calls.to_vec() };
        let output = self.build_and_call(call, None).await?;
        let tryBlockAndAggregateReturn { blockNumber, blockHash, returnData } = output;
        Ok((blockNumber.to::<u64>(), blockHash, T::decode_return_results(&returnData)?))
    }

    /// Helper fn to build a tx and call the multicall contract
    ///
    /// ## Params
    ///
    /// - `call_type`: The [`SolCall`] being made.
    /// - `value`: Total value to send with the call in case of `aggregate3Value` request.
    async fn build_and_call<M: SolCall>(
        &self,
        call_type: M,
        value: Option<U256>,
    ) -> Result<M::Return> {
        let call = call_type.abi_encode();
        let mut tx = N::TransactionRequest::default()
            .with_to(self.address)
            .with_input(Bytes::from_iter(call));

        if let Some(value) = value {
            tx.set_value(value);
        }

        let mut eth_call = self.provider.root().call(&tx);

        if let Some(block) = self.block {
            eth_call = eth_call.block(block);
        }

        if let Some(overrides) = &self.state_override {
            eth_call = eth_call.overrides(overrides);
        }

        let res = eth_call.await.map_err(MulticallError::TransportError)?;
        M::abi_decode_returns(&res, false).map_err(MulticallError::DecodeError)
    }

    /// Add a call to get the block hash from a block number
    pub fn get_block_hash(self, number: BlockNumber) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getBlockHashCall>,
        T::Pushed: CallTuple,
    {
        let call = CallItem::<getBlockHashCall>::new(
            self.address,
            getBlockHashCall { blockNumber: U256::from(number) }.abi_encode().into(),
        );
        self.add_call(call)
    }

    /// Add a call to get the coinbase of the current block
    pub fn get_current_block_coinbase(self) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getCurrentBlockCoinbaseCall>,
        T::Pushed: CallTuple,
    {
        let call = CallItem::<getCurrentBlockCoinbaseCall>::new(
            self.address,
            getCurrentBlockCoinbaseCall {}.abi_encode().into(),
        );
        self.add_call(call)
    }

    /// Add a call to get the current block number
    pub fn get_block_number(self) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getBlockNumberCall>,
        T::Pushed: CallTuple,
    {
        let call = CallItem::<getBlockNumberCall>::new(
            self.address,
            getBlockNumberCall {}.abi_encode().into(),
        );
        self.add_call(call)
    }

    /// Add a call to get the current block difficulty
    pub fn get_current_block_difficulty(self) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getCurrentBlockDifficultyCall>,
        T::Pushed: CallTuple,
    {
        let call = CallItem::<getCurrentBlockDifficultyCall>::new(
            self.address,
            getCurrentBlockDifficultyCall {}.abi_encode().into(),
        );
        self.add_call(call)
    }

    /// Add a call to get the current block gas limit
    pub fn get_current_block_gas_limit(self) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getCurrentBlockGasLimitCall>,
        T::Pushed: CallTuple,
    {
        let call = CallItem::<getCurrentBlockGasLimitCall>::new(
            self.address,
            getCurrentBlockGasLimitCall {}.abi_encode().into(),
        );
        self.add_call(call)
    }

    /// Add a call to get the current block timestamp
    pub fn get_current_block_timestamp(self) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getCurrentBlockTimestampCall>,
        T::Pushed: CallTuple,
    {
        let call = CallItem::<getCurrentBlockTimestampCall>::new(
            self.address,
            getCurrentBlockTimestampCall {}.abi_encode().into(),
        );
        self.add_call(call)
    }

    /// Add a call to get the chain id
    pub fn get_chain_id(self) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getChainIdCall>,
        T::Pushed: CallTuple,
    {
        let call =
            CallItem::<getChainIdCall>::new(self.address, getChainIdCall {}.abi_encode().into());
        self.add_call(call)
    }

    /// Add a call to get the base fee
    pub fn get_base_fee(self) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getBasefeeCall>,
        T::Pushed: CallTuple,
    {
        let call =
            CallItem::<getBasefeeCall>::new(self.address, getBasefeeCall {}.abi_encode().into());
        self.add_call(call)
    }

    /// Add a call to get the eth balance of an address
    pub fn get_eth_balance(self, address: Address) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getEthBalanceCall>,
        T::Pushed: CallTuple,
    {
        let call = CallItem::<getEthBalanceCall>::new(
            self.address,
            getEthBalanceCall { addr: address }.abi_encode().into(),
        );
        self.add_call(call)
    }

    /// Add a call to get the last block hash
    pub fn get_last_block_hash(self) -> MulticallBuilder<T::Pushed, P, N>
    where
        T: TuplePush<getLastBlockHashCall>,
        T::Pushed: CallTuple,
    {
        let call = CallItem::<getLastBlockHashCall>::new(
            self.address,
            getLastBlockHashCall {}.abi_encode().into(),
        );
        self.add_call(call)
    }

    /// Returns an [`Empty`] builder
    ///
    /// Retains previously set provider, address, block and state_override settings.
    pub fn clear(self) -> MulticallBuilder<Empty, P, N> {
        MulticallBuilder {
            calls: Vec::new(),
            provider: self.provider,
            block: self.block,
            state_override: self.state_override,
            address: self.address,
            _pd: Default::default(),
        }
    }

    /// Get the number of calls in the builder
    pub fn len(&self) -> usize {
        self.calls.len()
    }

    /// Check if the builder is empty
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
}
