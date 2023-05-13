//! Foundry's procedural macros.
//! 
//! Also includes traits and other utilities used by the macros.

pub mod fmt;
pub use fmt::{console_format, ConsoleFmt, FormatSpec, TokenDisplay, UIfmt};

#[doc(inline)]
pub use foundry_macros_impl::ConsoleFmt;
