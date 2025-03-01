//! Conditional aliases to numeric types.
//!
//! # Examples
//!
//! ```
//! use std::fs::File;
//! use condtype::num::Usize64;
//!
//! fn handle_throughput(bytes: Usize64) {
//!     // ...
//! }
//!
//! // usize
//! let s: &str = // ...
//!     # "";
//! handle_throughput(s.len() as Usize64);
//!
//! // u64
//! # fn file() -> std::io::Result<()> {
//! let f: File = // ...
//! # panic!();
//! handle_throughput(f.metadata()?.len() as Usize64);
//! # Ok(()) }
//! ```
//!
//! # Pitfalls
//!
//! Because these are type aliases, some operations may happen to work for the
//! current target but not for other targets.
//!
//! The following example handles [`Usize32`] explicitly as [`usize`] or [`u32`]
//! depending on whether the platform is 64-bit or 32-bit:
//!
//! ```
//! # use condtype::num::Usize32;
//! #[cfg(target_pointer_width = "64")]
//! let x: Usize32 = usize::MAX;
//!
//! #[cfg(target_pointer_width = "32")]
//! let x: Usize32 = u32::MAX;
//! ```
//!
//! Instead, the code should be made portable by using an `as` cast:
//!
//! ```
//! # use condtype::num::Usize32;
//! let x: Usize32 = usize::MAX as Usize32;
//! ```

use core::mem::size_of;

use crate::CondType;

/// Conditional alias to the larger of two types.
macro_rules! max_ty {
    ($a:ty, $b:ty) => {
        CondType<{ size_of::<$a>() > size_of::<$b>() }, $a, $b>
    };
}

/// A signed integer that is convertible from [`isize`] and [`i8`]–[`i16`].
///
/// The integer is guaranteed to be at least [`isize`] large and [`i16`] small.
pub type Isize16 = max_ty!(isize, i16);

/// A signed integer that is convertible from [`isize`] and [`i8`]–[`i32`].
///
/// The integer is guaranteed to be at least [`isize`] large and [`i32`] small.
pub type Isize32 = max_ty!(isize, i32);

/// A signed integer that is convertible from [`isize`] and [`i8`]–[`i64`].
///
/// The integer is guaranteed to be at least [`isize`] large and [`i64`] small.
pub type Isize64 = max_ty!(isize, i64);

/// A signed integer that is convertible from [`isize`] and [`i8`]–[`i128`].
///
/// The integer is guaranteed to be at least [`isize`] large and [`i128`] small.
pub type Isize128 = max_ty!(isize, i128);

/// An unsigned integer that is convertible from [`usize`] and [`u8`]–[`u16`].
///
/// The integer is guaranteed to be at least [`usize`] large and [`u16`] small.
pub type Usize16 = max_ty!(usize, u16);

/// An unsigned integer that is convertible from [`usize`] and [`u8`]–[`u32`].
///
/// The integer is guaranteed to be at least [`usize`] large and [`u32`] small.
pub type Usize32 = max_ty!(usize, u32);

/// An unsigned integer that is convertible from [`usize`] and [`u8`]–[`u64`].
///
/// The integer is guaranteed to be at least [`usize`] large and [`u64`] small.
pub type Usize64 = max_ty!(usize, u64);

/// An unsigned integer that is convertible from [`usize`] and [`u8`]–[`u128`].
///
/// The integer is guaranteed to be at least [`usize`] large and [`u128`] small.
pub type Usize128 = max_ty!(usize, u128);

#[cfg(test)]
mod tests {
    use core::any::{type_name, TypeId};

    use super::*;

    #[test]
    fn expected_type() {
        macro_rules! assert_eq_ty {
            ($a:ty, $b:ty) => {
                assert_eq!(
                    TypeId::of::<$a>(),
                    TypeId::of::<$b>(),
                    "{} != {}",
                    type_name::<$a>(),
                    type_name::<$b>()
                );
            };
        }

        #[cfg(target_pointer_width = "32")]
        assert_eq_ty!(Usize32, u32);

        #[cfg(target_pointer_width = "64")]
        assert_eq_ty!(Usize32, usize);

        #[cfg(any(target_pointer_width = "32", target_pointer_width = "64"))]
        assert_eq_ty!(Usize64, u64);

        #[cfg(any(target_pointer_width = "32", target_pointer_width = "64"))]
        assert_eq_ty!(Usize128, u128);
    }
}
