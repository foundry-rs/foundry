//! [`Value`] and friends: types representing valid configuration values.
//!
mod value;
mod ser;
mod de;
mod tag;

#[cfg(feature = "parse-value")]
mod parse;

#[cfg(feature = "parse-value")]
mod escape;

pub mod magic;

pub(crate) use {self::ser::*, self::de::*};

pub use tag::Tag;
pub use value::{Value, Map, Num, Dict, Empty};
pub use uncased::{Uncased, UncasedStr};
