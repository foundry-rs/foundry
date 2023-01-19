//! Foundry's procedural macros.

// TODO: Remove dependency on foundry-common (currently only used for the UIfmt trait).

mod console_fmt;
pub use console_fmt::{console_format, ConsoleFmt, FormatSpec};

pub use foundry_macros_impl::ConsoleFmt;
