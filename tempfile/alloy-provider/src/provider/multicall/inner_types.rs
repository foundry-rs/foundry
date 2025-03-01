use std::{fmt::Debug, marker::PhantomData};

use super::{
    bindings::IMulticall3::{Call, Call3, Call3Value},
    CallTuple,
};
use alloy_primitives::{Address, Bytes, U256};
use alloy_sol_types::SolCall;
use thiserror::Error;

/// Result type for multicall operations.
pub type Result<T, E = MulticallError> = core::result::Result<T, E>;

/// A struct representing a failure in a multicall
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Call failed at index {idx} with return data: {return_data:?}")]
pub struct Failure {
    /// The index-position of the call that failed
    pub idx: usize,
    /// The return data of the call that failed
    pub return_data: Bytes,
}

/// A trait that is to be implemented by a type that can be distilled to a singular contract call
/// item.
pub trait MulticallItem {
    /// Decoder for the return data of the call.
    type Decoder: SolCall;

    /// The target address of the call.
    fn target(&self) -> Address;
    /// ABI-encoded input data for the call.
    fn input(&self) -> Bytes;
}

/// Helper type to build a [`CallItem`]
#[derive(Debug)]
pub struct CallItemBuilder;

impl CallItemBuilder {
    /// Create a new [`CallItem`] instance.
    #[allow(clippy::new_ret_no_self)]
    pub fn new<Item: MulticallItem>(item: Item) -> CallItem<Item::Decoder> {
        CallItem::new(item.target(), item.input())
    }
}

/// A singular call type that is mapped into aggregate, aggregate3, aggregate3Value call structs via
/// the [`CallInfoTrait`] trait.
#[derive(Clone)]
pub struct CallItem<D: SolCall> {
    target: Address,
    input: Bytes,
    allow_failure: bool,
    value: U256,
    decoder: PhantomData<D>,
}

impl<D: SolCall> Debug for CallItem<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallItem")
            .field("target", &self.target)
            .field("allow_failure", &self.allow_failure)
            .field("value", &self.value)
            .field("input", &self.input)
            .finish()
    }
}

impl<D: SolCall> CallItem<D> {
    /// Create a new [`CallItem`] instance.
    pub fn new(target: Address, input: Bytes) -> Self {
        Self { target, input, allow_failure: false, value: U256::ZERO, decoder: PhantomData }
    }

    /// Set whether the call should be allowed to fail or not.
    pub fn allow_failure(mut self, allow_failure: bool) -> Self {
        self.allow_failure = allow_failure;
        self
    }

    /// Set the value to send with the call.
    pub fn value(mut self, value: U256) -> Self {
        self.value = value;
        self
    }
}
impl<D: SolCall> CallInfoTrait for CallItem<D> {
    fn to_call(&self) -> Call {
        Call { target: self.target, callData: self.input.clone() }
    }

    fn to_call3(&self) -> Call3 {
        Call3 {
            target: self.target,
            allowFailure: self.allow_failure,
            callData: self.input.clone(),
        }
    }

    fn to_call3_value(&self) -> Call3Value {
        Call3Value {
            target: self.target,
            allowFailure: self.allow_failure,
            callData: self.input.clone(),
            value: self.value,
        }
    }
}
/// A trait for converting CallItem into relevant call types.
pub trait CallInfoTrait: std::fmt::Debug {
    /// Converts the [`CallItem`] into a [`Call`] struct for `aggregateCall`
    fn to_call(&self) -> Call;
    /// Converts the [`CallItem`] into a [`Call3`] struct for `aggregate3Call`
    fn to_call3(&self) -> Call3;
    /// Converts the [`CallItem`] into a [`Call3Value`] struct for `aggregate3Call`
    fn to_call3_value(&self) -> Call3Value;
}

/// Marker for Dynamic Calls i.e where in SolCall type is locked to one specific type and multicall
/// returns a Vec of the corresponding return type instead of a tuple.
#[derive(Debug)]
pub struct Dynamic<D: SolCall>(PhantomData<fn(D) -> D>);

impl<D: SolCall> CallTuple for Dynamic<D> {
    type Returns = Vec<Result<D::Return, Failure>>;
    type SuccessReturns = Vec<D::Return>;

    fn decode_returns(data: &[Bytes]) -> Result<Self::SuccessReturns> {
        data.iter()
            .map(|d| D::abi_decode_returns(d, false).map_err(MulticallError::DecodeError))
            .collect()
    }

    fn decode_return_results(
        results: &[super::bindings::IMulticall3::Result],
    ) -> Result<Self::Returns> {
        let mut ret = vec![];
        for (idx, res) in results.iter().enumerate() {
            if res.success {
                ret.push(Ok(D::abi_decode_returns(&res.returnData, false)
                    .map_err(MulticallError::DecodeError)?));
            } else {
                ret.push(Err(Failure { idx, return_data: res.returnData.clone() }));
            }
        }

        Ok(ret)
    }

    fn try_into_success(results: Self::Returns) -> Result<Self::SuccessReturns> {
        let mut ret = vec![];
        for res in results {
            ret.push(res.map_err(|e| MulticallError::CallFailed(e.return_data))?);
        }
        Ok(ret)
    }
}

/// Multicall errors.
#[derive(Debug, Error)]
pub enum MulticallError {
    /// Encountered when an `aggregate/aggregate3` batch contains a transaction with a value.
    #[error("batch contains a tx with a value, try using .send() instead")]
    ValueTx,
    /// Error decoding return data.
    #[error("could not decode")]
    DecodeError(alloy_sol_types::Error),
    /// No return data was found.
    #[error("no return data")]
    NoReturnData,
    /// Call failed.
    #[error("call failed when success was assured, this occurs when try_into_success is called on a failed call")]
    CallFailed(Bytes),
    /// Encountered when a transport error occurs while calling a multicall batch.
    #[error("Transport error: {0}")]
    TransportError(#[from] alloy_transport::TransportError),
}
