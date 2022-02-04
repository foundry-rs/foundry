//! helper macros

macro_rules! forward_backend_method {
    ($name: ident (&self $(, $arg: ident : $arg_type: ty)* ) -> $return_type: ty) => {
        fn $name (&self $(, $arg : $arg_type)* ) -> $return_type {
            self.backend.$name( $($arg),* )
        }
    };
}

/// A helper macro to delegate backend calls that delegates functions to the `self.backend` filed
///
/// # Example
///
/// ```ignore
///  forward_backend_methods! {
///     gas_price() -> U256
///  }
/// ```
///
/// expands to
///
/// ```ignore
///  fn gas_price(&self) -> U256 {self.backend.gas_price()}
/// ```
///
/// # Example
///
/// Full delegation to `self.backend`
///
/// ```ignore
/// struct DelegateBackend<B> {
///     backend: B,
/// }
/// impl<B: Backend> for DelegateBackend<B> {
///    forward_backend_methods! {
///        gas_price() -> U256,
///        origin() -> H160,
///        block_hash(number: U256) -> H256,
///        block_number() -> U256,
///        block_coinbase() -> H160,
///        block_timestamp() -> U256,
///        block_difficulty() -> U256,
///        block_gas_limit() -> U256,
///        block_base_fee_per_gas() -> U256,
///        chain_id() -> U256,
///        exists(address: H160) -> bool,
///        basic(address: H160) -> Basic,
///        code(address: H160) -> Vec<u8>,
///        storage(address: H160, index: H256) -> H256,
///        original_storage(address: H160, index: H256) -> Option<H256>
///    }
/// }
/// ```
macro_rules! forward_backend_methods {
    ( $($name: ident ($($arg: ident : $arg_type: ty),* $(,)? ) -> $return_type: ty),* ) => {
        $(
            $crate::sputnik::macros::forward_backend_method!($name(&self $(, $arg : $arg_type)*) -> $return_type);
        )*
    };
}

pub(crate) use forward_backend_method;
pub(crate) use forward_backend_methods;
