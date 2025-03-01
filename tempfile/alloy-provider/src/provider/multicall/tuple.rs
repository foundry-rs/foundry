use super::{
    bindings::IMulticall3::Result as MulticallResult,
    inner_types::{Dynamic, Failure, MulticallError, Result},
};
use alloy_primitives::Bytes;
use alloy_sol_types::SolCall;

/// Sealed trait to prevent external implementations
mod private {
    #[allow(unnameable_types)]
    pub trait Sealed {}
}
use private::Sealed;

/// A trait for tuples that can have types pushed to them
#[doc(hidden)]
#[allow(unnameable_types)]
pub trait TuplePush<T> {
    /// The resulting type after pushing T
    type Pushed;
}

/// A trait for tuples of SolCalls that can be decoded
#[doc(hidden)]
pub trait CallTuple: Sealed {
    /// Flattened tuple consisting of the return values of each call.
    ///
    /// Each return value is wrapped in a [`Result`] in order to account for failures in calls when
    /// others succeed.
    ///
    /// - [`Result::Ok`] contains the decoded return value of the call.
    /// - [`Result::Err`] contains a [`Failure`] struct with the index of the call that failed and
    ///   the raw bytes returned on failure.
    ///
    /// For example,
    ///
    /// ```ignore
    /// use alloy_sol_types::sol;
    /// use alloy_primitives::Address;
    /// use alloy_provider::{CallItem, Provider, ProviderBuilder, Result, Failure};
    /// use crate::SomeContract::failureCall;
    /// sol! {
    ///     #[derive(Debug)]
    ///     #[sol(rpc)]
    ///     contract SomeContract {
    ///       function success() external;
    ///       function failure() external;
    ///     }
    /// }
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let target = Address::random();
    ///     let provider = ProviderBuilder::new().on_builtin("https://..").await.unwrap();    
    ///     let some_contract = SomeContract::new(target, &provider);
    ///     let allow_failure_call = CallItem::<failureCall>::new(target, some_contract.failure().input()).allow_failure(true); // This calls is allowed to fail so that the batch doesn't revert.
    ///
    ///     let multicall = provider.multicall().add(some_contract.success()).add_call(allow_failure_call);
    ///
    ///     let (success_result, failure_result) = multicall.aggregate3().await.unwrap();
    ///     match success_result {
    ///       Ok(success) => { println!("Success: {:?}", success) },
    ///       Err(failure) => { /* handle failure */ },
    ///     }
    ///
    ///     match failure_result {
    ///       Ok(success) => { /* handle success */ },
    ///       Err(failure) => { assert!(matches!(failure, Failure { idx: 1, return_data: _ })) },
    ///     }
    /// }
    /// ```
    type Returns;

    /// Flattened tuple consisting of the decoded return values of each call.
    type SuccessReturns;

    /// Decode the returns from a sequence of bytes
    ///
    /// To be used for calls where success is ensured i.e `allow_failure` for all calls is false.
    fn decode_returns(data: &[Bytes]) -> Result<Self::SuccessReturns>;

    /// Converts Returns to SuccessReturns if all results are Ok
    fn decode_return_results(results: &[MulticallResult]) -> Result<Self::Returns>;

    /// Converts Returns to SuccessReturns if all results are Ok
    fn try_into_success(results: Self::Returns) -> Result<Self::SuccessReturns>;
}

/// Type indicating that the [`MulticallBuilder`](crate::MulticallBuilder) is empty.
#[derive(Debug, Clone)]
pub struct Empty;

impl Sealed for Empty {}

impl<T: SolCall> TuplePush<T> for Empty {
    type Pushed = (T,);
}

impl CallTuple for Empty {
    type Returns = ();
    type SuccessReturns = ();
    fn decode_returns(_: &[Bytes]) -> Result<Self::SuccessReturns> {
        Ok(())
    }
    fn decode_return_results(_results: &[MulticallResult]) -> Result<Self::Returns> {
        Ok(())
    }
    fn try_into_success(_: Self::Returns) -> Result<Self::SuccessReturns> {
        Ok(())
    }
}

impl<D: SolCall> Sealed for Dynamic<D> {}

// Macro to implement for tuples of different sizes
macro_rules! impl_tuple {
    ($($idx:tt => $ty:ident),+) => {
        impl<$($ty: SolCall,)+> Sealed for ($($ty,)+) {}

        // Implement pushing a new type onto the tuple
        impl<T: SolCall, $($ty: SolCall,)+> TuplePush<T> for ($($ty,)+) {
            type Pushed = ($($ty,)+ T,);
        }

        // Implement decoding for the tuple
        impl<$($ty: SolCall,)+> CallTuple for ($($ty,)+) {
            // The Returns associated type is a tuple of each SolCall's Return type
            type Returns = ($(Result<$ty::Return, Failure>,)+);

            type SuccessReturns = ($($ty::Return,)+);

            fn decode_returns(data: &[Bytes]) -> Result<Self::SuccessReturns> {
                if data.len() != count!($($ty),+) {
                    return Err(MulticallError::NoReturnData);
                }

                // Decode each return value in order
                Ok(($($ty::abi_decode_returns(&data[$idx], false).map_err(MulticallError::DecodeError)?,)+))
            }

            fn decode_return_results(results: &[MulticallResult]) -> Result<Self::Returns> {
                if results.len() != count!($($ty),+) {
                    return Err(MulticallError::NoReturnData);
                }

                Ok(($(
                    match &results[$idx].success {
                        true => Ok($ty::abi_decode_returns(&results[$idx].returnData, false).map_err(MulticallError::DecodeError)?),
                        false => Err(Failure { idx: $idx, return_data: results[$idx].returnData.clone() }),
                    },
                )+))
            }

            fn try_into_success(results: Self::Returns) -> Result<Self::SuccessReturns> {
                Ok(($(
                    match results.$idx {
                        Ok(value) => value,
                        Err(failure) => return Err(MulticallError::CallFailed(failure.return_data)),
                    },
                )+))
            }
        }
    };
}

// Helper macro to count number of types
macro_rules! count {
    () => (0);
    ($x:tt $(,$xs:tt)*) => (1 + count!($($xs),*));
}

// Max CALL_LIMIT is 16
impl_tuple!(0 => T1);
impl_tuple!(0 => T1, 1 => T2);
impl_tuple!(0 => T1, 1 => T2, 2 => T3);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10, 10 => T11);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10, 10 => T11, 11 => T12);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10, 10 => T11, 11 => T12, 12 => T13);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10, 10 => T11, 11 => T12, 12 => T13, 13 => T14);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10, 10 => T11, 11 => T12, 12 => T13, 13 => T14, 14 => T15);
impl_tuple!(0 => T1, 1 => T2, 2 => T3, 3 => T4, 4 => T5, 5 => T6, 6 => T7, 7 => T8, 8 => T9, 9 => T10, 10 => T11, 11 => T12, 12 => T13, 13 => T14, 14 => T15, 15 => T16);
