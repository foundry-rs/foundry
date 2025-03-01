//! # futures-utils-wasm
//!
//! A very simple crate for future-related utilities to abstract the differences between wasm and
//! non-wasm futures, namely the `Send` bound.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(all(feature = "alloc", not(feature = "std")))]
use alloc::boxed::Box;
use core::{future::Future, pin::Pin};

/// Macro that return an `impl Future` type, with a `Send` bound on non-wasm targets.
///
/// # Examples
///
/// ```
/// fn my_async_fn() -> futures_utils_wasm::impl_future!(<Output = ()>) {
///     async {}
/// }
/// ```
#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! impl_future {
    (<$($t:tt)+) => {
        impl Send + $crate::private::Future<$($t)+
    };
}

/// Macro that return an `impl Future` type, with a `Send` bound on non-wasm targets.
///
/// # Examples
///
/// ```
/// fn my_async_fn() -> futures_utils_wasm::impl_future!(<Output = ()>) {
///     async {}
/// }
/// ```
#[cfg(target_arch = "wasm32")]
#[macro_export]
macro_rules! impl_future {
    (<$($t:tt)+) => {
        impl $crate::private::Future<$($t)+
    };
}

/// Type alias for a pin-boxed future, with a `Send` bound on non-wasm targets.
#[cfg(all(feature = "alloc", not(target_arch = "wasm32")))]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a + Send>>;

/// Type alias for a pin-boxed future, with a `Send` bound on non-wasm targets.
#[cfg(all(feature = "alloc", target_arch = "wasm32"))]
#[cfg_attr(docsrs, doc(cfg(feature = "alloc")))]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

// Not public API.
#[doc(hidden)]
pub mod private {
    pub use core::future::Future;
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    fn _assert_future<T, F: Future<Output = T>>(_: F) {}

    fn _test_impl_future() {
        fn my_fn() -> impl_future!(<Output = ()>) {
            async {}
        }
        async fn my_async_fn() {}
        _assert_future(my_fn());
        _assert_future(my_async_fn());

        #[cfg(feature = "alloc")]
        {
            fn foo() -> BoxFuture<'static, ()> {
                Box::pin(async {})
            }
            _assert_future(foo());
        }
    }

    fn _test_impl_future_in_trait() {
        trait MyTrait {
            fn my_fn() -> impl_future!(<Output = ()>);
        }
        impl MyTrait for () {
            fn my_fn() -> impl_future!(<Output = ()>) {
                async {}
            }
        }
        impl MyTrait for u8 {
            async fn my_fn() {}
        }
        _assert_future(<()>::my_fn());
        _assert_future(<u8>::my_fn());
    }
}
