use crate::{CallDecoder, Error, EthCall, Result};
use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_network::{Ethereum, Network, TransactionBuilder, TransactionBuilder4844};
use alloy_network_primitives::ReceiptResponse;
use alloy_primitives::{Address, Bytes, ChainId, TxKind, U256};
use alloy_provider::{PendingTransactionBuilder, Provider};
use alloy_rpc_types_eth::{state::StateOverride, AccessList, BlobTransactionSidecar, BlockId};
use alloy_sol_types::SolCall;
use std::{self, marker::PhantomData};

// NOTE: The `T` generic here is kept to mitigate breakage with the `sol!` macro.
// It should always be `()` and has no effect on the implementation.

/// [`CallBuilder`] using a [`SolCall`] type as the call decoder.
// NOTE: please avoid changing this type due to its use in the `sol!` macro.
pub type SolCallBuilder<T, P, C, N = Ethereum> = CallBuilder<T, P, PhantomData<C>, N>;

/// [`CallBuilder`] using a [`Function`] as the call decoder.
pub type DynCallBuilder<T, P, N = Ethereum> = CallBuilder<T, P, Function, N>;

/// [`CallBuilder`] that does not have a call decoder.
pub type RawCallBuilder<T, P, N = Ethereum> = CallBuilder<T, P, (), N>;

/// A builder for sending a transaction via `eth_sendTransaction`, or calling a contract via
/// `eth_call`.
///
/// The builder can be `.await`ed directly, which is equivalent to invoking [`call`].
/// Prefer using [`call`] when possible, as `await`ing the builder directly will consume it, and
/// currently also boxes the future due to type system limitations.
///
/// A call builder can currently be instantiated in the following ways:
/// - by [`sol!`][sol]-generated contract structs' methods (through the `#[sol(rpc)]` attribute)
///   ([`SolCallBuilder`]);
/// - by [`ContractInstance`](crate::ContractInstance)'s methods ([`DynCallBuilder`]);
/// - using [`CallBuilder::new_raw`] ([`RawCallBuilder`]).
///
/// Each method represents a different way to decode the output of the contract call.
///
/// [`call`]: CallBuilder::call
///
/// # Note
///
/// This will set [state overrides](https://geth.ethereum.org/docs/rpc/ns-eth#3-object---state-override-set)
/// for `eth_call`, but this is not supported by all clients.
///
/// # Examples
///
/// Using [`sol!`][sol]:
///
/// ```no_run
/// # async fn test<P: alloy_provider::Provider>(provider: P) -> Result<(), Box<dyn std::error::Error>> {
/// use alloy_contract::SolCallBuilder;
/// use alloy_primitives::{Address, U256};
/// use alloy_sol_types::sol;
///
/// sol! {
///     #[sol(rpc)] // <-- Important!
///     contract MyContract {
///         function doStuff(uint a, bool b) public returns(address c, bytes32 d);
///     }
/// }
///
/// # stringify!(
/// let provider = ...;
/// # );
/// let address = Address::ZERO;
/// let contract = MyContract::new(address, &provider);
///
/// // Through `contract.<function_name>(args...)`
/// let a = U256::ZERO;
/// let b = true;
/// let builder: SolCallBuilder<_, _, MyContract::doStuffCall, _> = contract.doStuff(a, b);
/// let MyContract::doStuffReturn { c: _, d: _ } = builder.call().await?;
///
/// // Through `contract.call_builder(&<FunctionCall { args... }>)`:
/// // (note that this is discouraged because it's inherently less type-safe)
/// let call = MyContract::doStuffCall { a, b };
/// let builder: SolCallBuilder<_, _, MyContract::doStuffCall, _> = contract.call_builder(&call);
/// let MyContract::doStuffReturn { c: _, d: _ } = builder.call().await?;
/// # Ok(())
/// # }
/// ```
///
/// Using [`ContractInstance`](crate::ContractInstance):
///
/// ```no_run
/// # async fn test<P: alloy_provider::Provider>(provider: P, dynamic_abi: alloy_json_abi::JsonAbi) -> Result<(), Box<dyn std::error::Error>> {
/// use alloy_primitives::{Address, Bytes, U256};
/// use alloy_dyn_abi::DynSolValue;
/// use alloy_contract::{CallBuilder, ContractInstance, DynCallBuilder, Interface, RawCallBuilder};
///
/// # stringify!(
/// let dynamic_abi: JsonAbi = ...;
/// # );
/// let interface = Interface::new(dynamic_abi);
///
/// # stringify!(
/// let provider = ...;
/// # );
/// let address = Address::ZERO;
/// let contract: ContractInstance<_, _> = interface.connect(address, &provider);
///
/// // Build and call the function:
/// let call_builder: DynCallBuilder<(), _, _> = contract.function("doStuff", &[U256::ZERO.into(), true.into()])?;
/// let result: Vec<DynSolValue> = call_builder.call().await?;
///
/// // You can also decode the output manually. Get the raw bytes:
/// let raw_result: Bytes = call_builder.call_raw().await?;
/// // Or, equivalently:
/// let raw_builder: RawCallBuilder<(), _, _> = call_builder.clone().clear_decoder();
/// let raw_result: Bytes = raw_builder.call().await?;
/// // Decode the raw bytes:
/// let decoded_result: Vec<DynSolValue> = call_builder.decode_output(raw_result, false)?;
/// # Ok(())
/// # }
/// ```
///
/// [sol]: alloy_sol_types::sol
#[derive(Clone)]
#[must_use = "call builders do nothing unless you `.call`, `.send`, or `.await` them"]
pub struct CallBuilder<T, P, D, N: Network = Ethereum> {
    pub(crate) request: N::TransactionRequest,
    block: BlockId,
    state: Option<StateOverride>,
    /// The provider.
    // NOTE: This is public due to usage in `sol!`, please avoid changing it.
    pub provider: P,
    decoder: D,
    fake_transport: PhantomData<T>,
}

impl<T, P, D, N: Network> CallBuilder<T, P, D, N> {
    /// Converts the call builder to the inner transaction request
    pub fn into_transaction_request(self) -> N::TransactionRequest {
        self.request
    }
}

impl<T, P, D, N: Network> AsRef<N::TransactionRequest> for CallBuilder<T, P, D, N> {
    fn as_ref(&self) -> &N::TransactionRequest {
        &self.request
    }
}

// See [`ContractInstance`].
impl<T, P: Provider<N>, N: Network> DynCallBuilder<T, P, N> {
    pub(crate) fn new_dyn(
        provider: P,
        address: &Address,
        function: &Function,
        args: &[DynSolValue],
    ) -> Result<Self> {
        Ok(Self::new_inner_call(
            provider,
            function.abi_encode_input(args)?.into(),
            function.clone(),
        )
        .to(*address))
    }

    /// Clears the decoder, returning a raw call builder.
    #[inline]
    pub fn clear_decoder(self) -> RawCallBuilder<T, P, N> {
        RawCallBuilder {
            request: self.request,
            block: self.block,
            state: self.state,
            provider: self.provider,
            decoder: (),
            fake_transport: PhantomData,
        }
    }
}

#[doc(hidden)]
impl<'a, T, P: Provider<N>, C: SolCall, N: Network> SolCallBuilder<T, &'a P, C, N> {
    // `sol!` macro constructor, see `#[sol(rpc)]`. Not public API.
    // NOTE: please avoid changing this function due to its use in the `sol!` macro.
    pub fn new_sol(provider: &'a P, address: &Address, call: &C) -> Self {
        Self::new_inner_call(provider, call.abi_encode().into(), PhantomData::<C>).to(*address)
    }
}

impl<T, P: Provider<N>, C: SolCall, N: Network> SolCallBuilder<T, P, C, N> {
    /// Clears the decoder, returning a raw call builder.
    #[inline]
    pub fn clear_decoder(self) -> RawCallBuilder<T, P, N> {
        RawCallBuilder {
            request: self.request,
            block: self.block,
            state: self.state,
            provider: self.provider,
            decoder: (),
            fake_transport: PhantomData,
        }
    }
}

impl<T, P: Provider<N>, N: Network> RawCallBuilder<T, P, N> {
    /// Sets the decoder to the provided [`SolCall`].
    ///
    /// Converts the raw call builder into a sol call builder.
    ///
    /// Note that generally you would want to instantiate a sol call builder directly using the
    /// `sol!` macro, but this method is provided for flexibility, for example to convert a raw
    /// deploy call builder into a sol call builder.
    ///
    /// # Examples
    ///
    /// Decode a return value from a constructor:
    ///
    /// ```no_run
    /// # use alloy_sol_types::sol;
    /// sol! {
    ///     // NOTE: This contract is not meant to be deployed on-chain, but rather
    ///     // used in a static call with its creation code as the call data.
    ///     #[sol(rpc, bytecode = "34601457602a60e052600161010052604060e0f35b5f80fdfe")]
    ///     contract MyContract {
    ///         // The type returned by the constructor.
    ///         #[derive(Debug, PartialEq)]
    ///         struct MyStruct {
    ///             uint64 a;
    ///             bool b;
    ///         }
    ///
    ///         constructor() {
    ///             MyStruct memory s = MyStruct(42, true);
    ///             bytes memory returnData = abi.encode(s);
    ///             assembly {
    ///                 return(add(returnData, 0x20), mload(returnData))
    ///             }
    ///         }
    ///
    ///         // A shim that represents the return value of the constructor.
    ///         function constructorReturn() external view returns (MyStruct memory s);
    ///     }
    /// }
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # stringify!(
    /// let provider = ...;
    /// # );
    /// # let provider = alloy_provider::ProviderBuilder::new().on_anvil();
    /// let call_builder = MyContract::deploy_builder(&provider)
    ///     .with_sol_decoder::<MyContract::constructorReturnCall>();
    /// let result = call_builder.call().await?;
    /// assert_eq!(result.s, MyContract::MyStruct { a: 42, b: true });
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn with_sol_decoder<C: SolCall>(self) -> SolCallBuilder<T, P, C, N> {
        SolCallBuilder {
            request: self.request,
            block: self.block,
            state: self.state,
            provider: self.provider,
            decoder: PhantomData::<C>,
            fake_transport: PhantomData,
        }
    }
}

impl<T, P: Provider<N>, N: Network> RawCallBuilder<T, P, N> {
    /// Creates a new call builder with the provided provider and ABI encoded input.
    ///
    /// Will not decode the output of the call, meaning that [`call`](Self::call) will behave the
    /// same as [`call_raw`](Self::call_raw).
    #[inline]
    pub fn new_raw(provider: P, input: Bytes) -> Self {
        Self::new_inner_call(provider, input, ())
    }

    /// Creates a new call builder with the provided provider and contract deploy code.
    ///
    /// Will not decode the output of the call, meaning that [`call`](Self::call) will behave the
    /// same as [`call_raw`](Self::call_raw).
    // NOTE: please avoid changing this function due to its use in the `sol!` macro.
    pub fn new_raw_deploy(provider: P, input: Bytes) -> Self {
        Self::new_inner_deploy(provider, input, ())
    }
}

impl<T, P: Provider<N>, D: CallDecoder, N: Network> CallBuilder<T, P, D, N> {
    fn new_inner_deploy(provider: P, input: Bytes, decoder: D) -> Self {
        Self {
            request: <N::TransactionRequest>::default().with_deploy_code(input),
            decoder,
            provider,
            block: BlockId::default(),
            state: None,
            fake_transport: PhantomData,
        }
    }

    fn new_inner_call(provider: P, input: Bytes, decoder: D) -> Self {
        Self {
            request: <N::TransactionRequest>::default().with_input(input),
            decoder,
            provider,
            block: BlockId::default(),
            state: None,
            fake_transport: PhantomData,
        }
    }

    /// Sets the `chain_id` field in the transaction to the provided value
    pub fn chain_id(mut self, chain_id: ChainId) -> Self {
        self.request.set_chain_id(chain_id);
        self
    }

    /// Sets the `from` field in the transaction to the provided value.
    pub fn from(mut self, from: Address) -> Self {
        self.request.set_from(from);
        self
    }

    /// Sets the transaction request to the provided tx kind.
    pub fn kind(mut self, to: TxKind) -> Self {
        self.request.set_kind(to);
        self
    }

    /// Sets the `to` field in the transaction to the provided address.
    pub fn to(mut self, to: Address) -> Self {
        self.request.set_to(to);
        self
    }

    /// Sets the `sidecar` field in the transaction to the provided value.
    pub fn sidecar(mut self, blob_sidecar: BlobTransactionSidecar) -> Self
    where
        N::TransactionRequest: TransactionBuilder4844,
    {
        self.request.set_blob_sidecar(blob_sidecar);
        self
    }

    /// Uses a Legacy transaction instead of an EIP-1559 one to execute the call
    pub fn legacy(self) -> Self {
        todo!()
    }

    /// Sets the `gas` field in the transaction to the provided value
    pub fn gas(mut self, gas: u64) -> Self {
        self.request.set_gas_limit(gas);
        self
    }

    /// Sets the `gas_price` field in the transaction to the provided value
    /// If the internal transaction is an EIP-1559 one, then it sets both
    /// `max_fee_per_gas` and `max_priority_fee_per_gas` to the same value
    pub fn gas_price(mut self, gas_price: u128) -> Self {
        self.request.set_gas_price(gas_price);
        self
    }

    /// Sets the `max_fee_per_gas` in the transaction to the provide value
    pub fn max_fee_per_gas(mut self, max_fee_per_gas: u128) -> Self {
        self.request.set_max_fee_per_gas(max_fee_per_gas);
        self
    }

    /// Sets the `max_priority_fee_per_gas` in the transaction to the provide value
    pub fn max_priority_fee_per_gas(mut self, max_priority_fee_per_gas: u128) -> Self {
        self.request.set_max_priority_fee_per_gas(max_priority_fee_per_gas);
        self
    }

    /// Sets the `max_fee_per_blob_gas` in the transaction to the provided value
    pub fn max_fee_per_blob_gas(mut self, max_fee_per_blob_gas: u128) -> Self
    where
        N::TransactionRequest: TransactionBuilder4844,
    {
        self.request.set_max_fee_per_blob_gas(max_fee_per_blob_gas);
        self
    }

    /// Sets the `access_list` in the transaction to the provided value
    pub fn access_list(mut self, access_list: AccessList) -> Self {
        self.request.set_access_list(access_list);
        self
    }

    /// Sets the `value` field in the transaction to the provided value
    pub fn value(mut self, value: U256) -> Self {
        self.request.set_value(value);
        self
    }

    /// Sets the `nonce` field in the transaction to the provided value
    pub fn nonce(mut self, nonce: u64) -> Self {
        self.request.set_nonce(nonce);
        self
    }

    /// Applies a function to the internal transaction request.
    pub fn map<F>(mut self, f: F) -> Self
    where
        F: FnOnce(N::TransactionRequest) -> N::TransactionRequest,
    {
        self.request = f(self.request);
        self
    }

    /// Sets the `block` field for sending the tx to the chain
    pub const fn block(mut self, block: BlockId) -> Self {
        self.block = block;
        self
    }

    /// Sets the [state override set](https://geth.ethereum.org/docs/rpc/ns-eth#3-object---state-override-set).
    ///
    /// # Note
    ///
    /// Not all client implementations will support this as a parameter to `eth_call`.
    pub fn state(mut self, state: StateOverride) -> Self {
        self.state = Some(state);
        self
    }

    /// Returns the underlying transaction's ABI-encoded data.
    pub fn calldata(&self) -> &Bytes {
        self.request.input().expect("set in the constructor")
    }

    /// Returns the estimated gas cost for the underlying transaction to be executed
    /// If [`state overrides`](Self::state) are set, they will be applied to the gas estimation.
    pub async fn estimate_gas(&self) -> Result<u64> {
        let mut estimate = self.provider.estimate_gas(&self.request);
        if let Some(state) = &self.state {
            estimate = estimate.overrides(state);
        }
        estimate.block(self.block).await.map_err(Into::into)
    }

    /// Queries the blockchain via an `eth_call` without submitting a transaction to the network.
    /// If [`state overrides`](Self::state) are set, they will be applied to the call.
    ///
    /// Returns the decoded the output by using the provided decoder.
    /// If this is not desired, use [`call_raw`](Self::call_raw) to get the raw output data.
    #[doc(alias = "eth_call")]
    #[doc(alias = "call_with_overrides")]
    pub fn call(&self) -> EthCall<'_, '_, D, N> {
        self.call_raw().with_decoder(&self.decoder)
    }

    /// Queries the blockchain via an `eth_call` without submitting a transaction to the network.
    /// If [`state overrides`](Self::state) are set, they will be applied to the call.
    ///
    /// Does not decode the output of the call, returning the raw output data instead.
    ///
    /// See [`call`](Self::call) for more information.
    pub fn call_raw(&self) -> EthCall<'_, '_, (), N> {
        let call = self.provider.call(&self.request).block(self.block);
        let call = match &self.state {
            Some(state) => call.overrides(state),
            None => call,
        };
        call.into()
    }

    /// Decodes the output of a contract function using the provided decoder.
    #[inline]
    pub fn decode_output(&self, data: Bytes, validate: bool) -> Result<D::CallOutput> {
        self.decoder.abi_decode_output(data, validate)
    }

    /// Broadcasts the underlying transaction to the network as a deployment transaction, returning
    /// the address of the deployed contract after the transaction has been confirmed.
    ///
    /// Returns an error if the transaction is not a deployment transaction, or if the contract
    /// address is not found in the deployment transaction’s receipt.
    ///
    /// For more fine-grained control over the deployment process, use [`send`](Self::send) instead.
    ///
    /// Note that the deployment address can be pre-calculated if the `from` address and `nonce` are
    /// known using [`calculate_create_address`](Self::calculate_create_address).
    pub async fn deploy(&self) -> Result<Address> {
        if !self.request.kind().is_some_and(|to| to.is_create()) {
            return Err(Error::NotADeploymentTransaction);
        }
        let pending_tx = self.send().await?;
        let receipt = pending_tx.get_receipt().await?;
        receipt.contract_address().ok_or(Error::ContractNotDeployed)
    }

    /// Broadcasts the underlying transaction to the network.
    ///
    /// Returns a builder for configuring the pending transaction watcher.
    /// See [`Provider::send_transaction`] for more information.
    pub async fn send(&self) -> Result<PendingTransactionBuilder<N>> {
        Ok(self.provider.send_transaction(self.request.clone()).await?)
    }

    /// Calculates the address that will be created by the transaction, if any.
    ///
    /// Returns `None` if the transaction is not a contract creation (the `to` field is set), or if
    /// the `from` or `nonce` fields are not set.
    pub fn calculate_create_address(&self) -> Option<Address> {
        self.request.calculate_create_address()
    }
}

impl<T, P: Clone, D, N: Network> CallBuilder<T, &P, D, N> {
    /// Clones the provider and returns a new builder with the cloned provider.
    pub fn with_cloned_provider(self) -> CallBuilder<T, P, D, N> {
        CallBuilder {
            request: self.request,
            block: self.block,
            state: self.state,
            provider: self.provider.clone(),
            decoder: self.decoder,
            fake_transport: PhantomData,
        }
    }
}

impl<T, P, D: CallDecoder, N: Network> std::fmt::Debug for CallBuilder<T, P, D, N> {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallBuilder")
            .field("request", &self.request)
            .field("block", &self.block)
            .field("state", &self.state)
            .field("decoder", &self.decoder.as_debug_field())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::Transaction;
    use alloy_primitives::{address, b256, bytes, hex, utils::parse_units, B256};
    use alloy_provider::{Provider, ProviderBuilder, WalletProvider};
    use alloy_rpc_types_eth::AccessListItem;
    use alloy_sol_types::sol;
    use futures::Future;

    #[test]
    fn empty_constructor() {
        sol! {
            #[sol(rpc, bytecode = "6942")]
            contract EmptyConstructor {
                constructor();
            }
        }

        let provider = ProviderBuilder::new().on_anvil();
        let call_builder = EmptyConstructor::deploy_builder(&provider);
        assert_eq!(*call_builder.calldata(), bytes!("6942"));
    }

    sol! {
        // Solc: 0.8.24+commit.e11b9ed9.Linux.g++
        // Command: solc a.sol --bin --via-ir --optimize --optimize-runs 1
        #[sol(rpc, bytecode = "60803461006357601f61014838819003918201601f19168301916001600160401b038311848410176100675780849260209460405283398101031261006357518015158091036100635760ff80195f54169116175f5560405160cc908161007c8239f35b5f80fd5b634e487b7160e01b5f52604160045260245ffdfe60808060405260043610156011575f80fd5b5f3560e01c9081638bf1799f14607a575063b09a261614602f575f80fd5b346076576040366003190112607657602435801515810360765715606f57604060015b81516004356001600160a01b0316815260ff919091166020820152f35b60405f6052565b5f80fd5b346076575f36600319011260765760209060ff5f541615158152f3fea264697066735822122043709781c9bdc30c530978abf5db25a4b4ccfebf989baafd2ba404519a7f7e8264736f6c63430008180033")]
        contract MyContract {
            bool public myState;

            constructor(bool myState_) {
                myState = myState_;
            }

            function doStuff(uint a, bool b) external pure returns(address c, bytes32 d) {
                return (address(uint160(a)), bytes32(uint256(b ? 1 : 0)));
            }
        }
    }

    sol! {
        // Solc: 0.8.24+commit.e11b9ed9.Linux.g++
        // Command: solc counter.sol --bin --via-ir --optimize --optimize-runs 1
        #[sol(rpc, bytecode = "608080604052346100155760d4908161001a8239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c90816361bc221a14607e575063d09de08a14602f575f80fd5b34607a575f366003190112607a575f546001600160801b038082166001018181116066576001600160801b03199092169116175f55005b634e487b7160e01b5f52601160045260245ffd5b5f80fd5b34607a575f366003190112607a575f546001600160801b03168152602090f3fea26469706673582212208b360e442c4bb2a4bbdec007ee24588c7a88e0aa52ac39efac748e5e23eff69064736f6c63430008180033")]
        contract Counter {
            uint128 public counter;

            function increment() external {
                counter += 1;
            }
        }
    }

    /// Creates a new call_builder to test field modifications, taken from [call_encoding]
    #[allow(clippy::type_complexity)]
    fn build_call_builder() -> CallBuilder<(), impl Provider, PhantomData<MyContract::doStuffCall>>
    {
        let provider = ProviderBuilder::new().on_anvil();
        let contract = MyContract::new(Address::ZERO, provider);
        let call_builder = contract.doStuff(U256::ZERO, true).with_cloned_provider();
        call_builder
    }

    #[test]
    fn change_chain_id() {
        let call_builder = build_call_builder().chain_id(1337);
        assert_eq!(
            call_builder.request.chain_id.expect("chain_id should be set"),
            1337,
            "chain_id of request should be '1337'"
        );
    }

    #[test]
    fn change_max_fee_per_gas() {
        let call_builder = build_call_builder().max_fee_per_gas(42);
        assert_eq!(
            call_builder.request.max_fee_per_gas.expect("max_fee_per_gas should be set"),
            42,
            "max_fee_per_gas of request should be '42'"
        );
    }

    #[test]
    fn change_max_priority_fee_per_gas() {
        let call_builder = build_call_builder().max_priority_fee_per_gas(45);
        assert_eq!(
            call_builder
                .request
                .max_priority_fee_per_gas
                .expect("max_priority_fee_per_gas should be set"),
            45,
            "max_priority_fee_per_gas of request should be '45'"
        );
    }

    #[test]
    fn change_max_fee_per_blob_gas() {
        let call_builder = build_call_builder().max_fee_per_blob_gas(50);
        assert_eq!(
            call_builder.request.max_fee_per_blob_gas.expect("max_fee_per_blob_gas should be set"),
            50,
            "max_fee_per_blob_gas of request should be '50'"
        );
    }

    #[test]
    fn change_access_list() {
        let access_list = AccessList::from(vec![AccessListItem {
            address: Address::ZERO,
            storage_keys: vec![B256::ZERO],
        }]);
        let call_builder = build_call_builder().access_list(access_list.clone());
        assert_eq!(
            call_builder.request.access_list.expect("access_list should be set"),
            access_list,
            "Access list of the transaction should have been set to our access list"
        )
    }

    #[test]
    fn call_encoding() {
        let provider = ProviderBuilder::new().on_anvil();
        let contract = MyContract::new(Address::ZERO, &&provider).with_cloned_provider();
        let call_builder = contract.doStuff(U256::ZERO, true).with_cloned_provider();
        assert_eq!(
            *call_builder.calldata(),
            bytes!(
                "b09a2616"
                "0000000000000000000000000000000000000000000000000000000000000000"
                "0000000000000000000000000000000000000000000000000000000000000001"
            ),
        );
        // Box the future to assert its concrete output type.
        let _future: Box<dyn Future<Output = Result<MyContract::doStuffReturn>> + Send> =
            Box::new(async move { call_builder.call().await });
    }

    #[test]
    fn deploy_encoding() {
        let provider = ProviderBuilder::new().on_anvil();
        let bytecode = &MyContract::BYTECODE[..];
        let call_builder = MyContract::deploy_builder(&provider, false);
        assert_eq!(
            call_builder.calldata()[..],
            [
                bytecode,
                &hex!("0000000000000000000000000000000000000000000000000000000000000000")[..]
            ]
            .concat(),
        );
        let call_builder = MyContract::deploy_builder(&provider, true);
        assert_eq!(
            call_builder.calldata()[..],
            [
                bytecode,
                &hex!("0000000000000000000000000000000000000000000000000000000000000001")[..]
            ]
            .concat(),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deploy_and_call() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        let expected_address = provider.default_signer_address().create(0);
        let my_contract = MyContract::deploy(provider, true).await.unwrap();
        assert_eq!(*my_contract.address(), expected_address);

        let my_state_builder = my_contract.myState();
        assert_eq!(my_state_builder.calldata()[..], MyContract::myStateCall {}.abi_encode(),);
        let result: MyContract::myStateReturn = my_state_builder.call().await.unwrap();
        assert!(result.myState);

        let do_stuff_builder = my_contract.doStuff(U256::from(0x69), true);
        assert_eq!(
            do_stuff_builder.calldata()[..],
            MyContract::doStuffCall { a: U256::from(0x69), b: true }.abi_encode(),
        );
        let result: MyContract::doStuffReturn = do_stuff_builder.call().await.unwrap();
        assert_eq!(result.c, address!("0000000000000000000000000000000000000069"));
        assert_eq!(
            result.d,
            b256!("0000000000000000000000000000000000000000000000000000000000000001"),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deploy_and_call_with_priority() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();
        let counter_contract = Counter::deploy(provider.clone()).await.unwrap();
        let max_fee_per_gas: U256 = parse_units("50", "gwei").unwrap().into();
        let max_priority_fee_per_gas: U256 = parse_units("0.1", "gwei").unwrap().into();
        let receipt = counter_contract
            .increment()
            .max_fee_per_gas(max_fee_per_gas.to())
            .max_priority_fee_per_gas(max_priority_fee_per_gas.to())
            .send()
            .await
            .expect("Could not send transaction")
            .get_receipt()
            .await
            .expect("Could not get the receipt");
        let transaction_hash = receipt.transaction_hash;
        let transaction = provider
            .get_transaction_by_hash(transaction_hash)
            .await
            .expect("failed to fetch tx")
            .expect("tx not included");
        assert_eq!(
            transaction.max_fee_per_gas(),
            max_fee_per_gas.to(),
            "max_fee_per_gas of the transaction should be set to the right value"
        );
        assert_eq!(
            transaction
                .max_priority_fee_per_gas()
                .expect("max_priority_fee_per_gas of the transaction should be set"),
            max_priority_fee_per_gas.to(),
            "max_priority_fee_per_gas of the transaction should be set to the right value"
        )
    }
}
