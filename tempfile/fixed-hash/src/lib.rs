// Copyright 2020 Parity Technologies
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![cfg_attr(not(feature = "std"), no_std)]

// Re-export liballoc using an alias so that the macros can work without
// requiring `extern crate alloc` downstream.
#[doc(hidden)]
pub extern crate alloc as alloc_;

// Re-export libcore using an alias so that the macros can work without
// requiring `use core` downstream.
#[doc(hidden)]
pub use core as core_;

// This disables a warning for unused #[macro_use(..)]
// which is incorrect since the compiler does not check
// for all available configurations.
#[allow(unused_imports)]
#[doc(hidden)]
pub use static_assertions;

// Export `const_assert` macro so that users of this crate do not
// have to import the `static_assertions` crate themselves.
#[doc(hidden)]
pub use static_assertions::const_assert;

#[cfg(feature = "byteorder")]
#[doc(hidden)]
pub use byteorder;

#[cfg(feature = "rustc-hex")]
#[doc(hidden)]
pub use rustc_hex;

#[cfg(feature = "rand")]
#[doc(hidden)]
pub use rand;

#[cfg(feature = "quickcheck")]
#[doc(hidden)]
pub use quickcheck;

#[cfg(feature = "arbitrary")]
#[doc(hidden)]
pub use arbitrary;

#[macro_use]
mod hash;

#[cfg(test)]
mod tests;

#[cfg(feature = "api-dummy")]
construct_fixed_hash! {
	/// Go here for an overview of the hash type API.
	pub struct ApiDummy(32);
}
