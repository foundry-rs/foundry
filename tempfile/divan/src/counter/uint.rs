use std::any::Any;

use crate::counter::{IntoCounter, ItemsCount};

/// The largest unsigned integer usable by counters provided by this crate.
///
/// If `usize > u64`, this is a type alias to `usize`. Otherwise, it is a type
/// alias to `u64`.
pub type MaxCountUInt = condtype::num::Usize64;

/// `u8`-`u64` and `usize`.
///
/// We deliberately do not implement this trait for `u128` to make it
/// impossible† to overflow `u128` when summing counts for averaging.
///
/// †When `usize` is larger than `u64`, it becomes possible to overflow `u128`.
/// In this case, Divan assumes
pub trait CountUInt: Copy + Any {
    fn into_max_uint(self) -> MaxCountUInt;
}

/// A type like `CountUInt` but with more options.
pub trait AsCountUInt {
    fn as_max_uint(&self) -> MaxCountUInt;
}

impl<T: AsCountUInt> AsCountUInt for &T {
    #[inline]
    fn as_max_uint(&self) -> MaxCountUInt {
        T::as_max_uint(self)
    }
}

macro_rules! impl_uint {
    ($($i:ty),+) => {
        $(impl CountUInt for $i {
            #[inline]
            fn into_max_uint(self) -> MaxCountUInt {
                self as _
            }
        })+

        $(impl AsCountUInt for $i {
            #[inline]
            fn as_max_uint(&self) -> MaxCountUInt {
                *self as _
            }
        })+

        $(impl IntoCounter for $i {
            type Counter = ItemsCount;

            #[inline]
            fn into_counter(self) -> ItemsCount {
                ItemsCount::new(self)
            }
        })+
    };
}

// These types must be losslessly convertible to `MaxCountUInt`.
impl_uint!(u8, u16, u32, u64, usize);
