//! EIP-2124 implementation based on <https://eips.ethereum.org/EIPS/eip-2124>.
//!
//! Previously version of Apache licenced [`ethereum-forkid`](https://crates.io/crates/ethereum-forkid).
#![cfg_attr(not(feature = "std"), no_std)]

#[allow(unused_imports)]
#[macro_use]
extern crate alloc;

mod head;
pub use head::Head;

mod forkid;
pub use forkid::*;
