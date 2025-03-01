#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
pub use std::{vec, vec::Vec};

#[cfg(not(feature = "std"))]
pub use alloc::{vec, vec::Vec};
