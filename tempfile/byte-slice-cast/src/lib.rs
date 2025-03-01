// Copyright (C) 2017,2018 Sebastian Dröge <sebastian@centricular.com>
//
// Licensed under the MIT license, see the LICENSE file or <http://opensource.org/licenses/MIT>

#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::missing_safety_doc)]

//! Safely cast bytes slices from/to slices of built-in fundamental numeric types.
//!
//! The provided traits here allow safe casting between byte slices and slices of fundamental
//! numeric types, like integers and floating point numbers. During casting, checks are performed
//! to ensure that the output slice is safe to use: the input slice must be properly aligned for
//! the output type and contain an integer number of values.
//!
//! Instead of working only on slices, the traits work on `AsRef<[T]>` in the immutable case and on
//! `AsMut<[T]>` for the mutable case. As such, it is possible to directly work on e.g. `Vec<T>`
//! and `Box<[T]>` too.
//!
//! The content of the output slice will be bitwise equivalent to the input slice, as such extra
//! care has to be taken with regard to endianness.
//!
//! # Example with slices
//! ```
//! # extern crate byte_slice_cast;
//! # fn main() {
//! use byte_slice_cast::*;
//!
//! let slice = [0x0102u16, 0x0304u16, 0x0506u16];
//!
//! let converted_slice = slice.as_byte_slice();
//!
//! if cfg!(target_endian = "big") {
//!     assert_eq!(converted_slice, &[1, 2, 3, 4, 5, 6]);
//! } else {
//!     assert_eq!(converted_slice, &[2, 1, 4, 3, 6, 5]);
//! }
//!
//! let converted_back_slice = converted_slice.as_slice_of::<u16>().unwrap();
//!
//! assert_eq!(converted_back_slice, &slice);
//! # }
//! ```
//!
//! # Example with mutable slices
//! ```
//! # extern crate byte_slice_cast;
//! # fn main() {
//! use byte_slice_cast::*;
//!
//! let mut slice = [0u32; 1];
//! let mut converted_slice = slice.as_mut_byte_slice();
//! converted_slice.copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
//!
//! let mut converted_slice = converted_slice.as_mut_slice_of::<u16>().unwrap();
//! converted_slice[0] = 0xffff;
//!
//! if cfg!(target_endian = "big") {
//!     assert_eq!(&slice, &[0xffff5678]);
//! } else {
//!     assert_eq!(&slice, &[0x7856ffff]);
//! }
//!
//! # }
//! ```

use core::{fmt, mem, slice};

#[cfg(feature = "std")]
use std::error::Error as StdError;

/// Possible errors during slice conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input slice is not properly aligned for the
    /// output data type. E.g. for an `u32` output slice
    /// the memory must be 4-byte aligned.
    AlignmentMismatch {
        dst_type: &'static str,
        dst_minimum_alignment: usize,
    },
    /// A non-integer number of values from the output
    /// type would be in the output slice.
    LengthMismatch {
        dst_type: &'static str,
        src_slice_size: usize,
        dst_type_size: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            Error::AlignmentMismatch {
                dst_type,
                dst_minimum_alignment,
            } => {
                write!(
                    f,
                    "cannot cast a &[u8] into a &[{}]: the slice's address is not divisible by the minimum alignment ({}) of {}",
                    dst_type,
                    dst_minimum_alignment,
                    dst_type
                )?;
            }
            Error::LengthMismatch {
                dst_type,
                src_slice_size,
                dst_type_size,
            } => {
                write!(
                    f,
                    "cannot cast a &[u8] into a &[{}]: the size ({}) of the slice is not divisible by the size ({}) of {}",
                    dst_type,
                    src_slice_size,
                    dst_type_size,
                    dst_type
                )?;
            }
        }

        Ok(())
    }
}

trait TypeName {
    const TYPE_NAME: &'static str;
}

#[cfg(feature = "std")]
impl StdError for Error {
    fn description(&self) -> &str {
        use self::Error::*;

        match *self {
            AlignmentMismatch { .. } => "Alignment Mismatch",
            LengthMismatch { .. } => "Length Mismatch",
        }
    }
}

fn check_alignment<T, U>(data: &T) -> Result<usize, Error>
where
    U: TypeName,
    T: AsRef<[u8]> + ?Sized,
{
    let alignment = mem::align_of::<U>();

    if (data.as_ref().as_ptr() as usize) % alignment != 0 {
        let err = Error::AlignmentMismatch {
            dst_type: U::TYPE_NAME,
            dst_minimum_alignment: alignment,
        };
        return Err(err);
    }
    Ok(alignment)
}

fn check_length<T, U>(data: &T) -> Result<usize, Error>
where
    U: TypeName,
    T: AsRef<[u8]> + ?Sized,
{
    let size_out = mem::size_of::<U>();
    if data.as_ref().len() % size_out != 0 {
        let err = Error::LengthMismatch {
            dst_type: U::TYPE_NAME,
            src_slice_size: data.as_ref().len(),
            dst_type_size: size_out,
        };
        return Err(err);
    }
    Ok(size_out)
}

fn check_constraints<U>(data: &[u8]) -> Result<usize, Error>
where
    U: TypeName,
{
    if data.is_empty() {
        return Ok(0);
    }

    check_alignment::<[u8], U>(data)?;
    let size_out = check_length::<[u8], U>(data)?;

    Ok(data.len() / size_out)
}

macro_rules! impl_trait(
    ($to:ty) => {
        impl TypeName for $to {
            const TYPE_NAME: &'static str = stringify!($to);
        }

        unsafe impl FromByteSlice for $to {
            #[inline]
            fn from_byte_slice<T: AsRef<[u8]> + ?Sized>(slice: &T) -> Result<&[$to], Error> {
                let slice = slice.as_ref();
                let len = check_constraints::<$to>(slice)?;

                // Need to handle the empty case separately as even an empty slices
                // must have a correctly aligned data pointer
                if len == 0 {
                    Ok(&[])
                } else {
                    #[allow(clippy::cast_ptr_alignment)]
                    unsafe {
                        Ok(slice::from_raw_parts(slice.as_ptr() as *const $to, len))
                    }
                }
            }

            #[inline]
            fn from_mut_byte_slice<T: AsMut<[u8]> + ?Sized>(slice: &mut T) -> Result<&mut [$to], Error> {
                let slice = slice.as_mut();
                let len = check_constraints::<$to>(slice)?;

                // Need to handle the empty case separately as even an empty slices
                // must have a correctly aligned data pointer
                if len == 0 {
                    Ok(&mut [])
                } else {
                    #[allow(clippy::cast_ptr_alignment)]
                    unsafe {
                        Ok(slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut $to, len))
                    }
                }
            }
        }

        unsafe impl ToByteSlice for $to {
            #[inline]
            fn to_byte_slice<T: AsRef<[$to]> + ?Sized>(slice: &T) -> &[u8] {
                let slice = slice.as_ref();
                let len = slice.len() * mem::size_of::<$to>();
                unsafe {
                    slice::from_raw_parts(slice.as_ptr() as *const u8, len)
                }
            }
        }

        unsafe impl ToMutByteSlice for $to {
            #[inline]
            fn to_mut_byte_slice<T: AsMut<[$to]> + ?Sized>(slice: &mut T) -> &mut [u8] {
                let slice = slice.as_mut();
                let len = slice.len() * mem::size_of::<$to>();
                unsafe {
                    slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut u8, len)
                }
            }
        }
    };
);

macro_rules! impl_trait_array (
    ($to:ty) => {
        impl<const N: usize> TypeName for [$to; N] {
            const TYPE_NAME: &'static str = stringify!([$to; N]);
        }

        unsafe impl<const N: usize> FromByteSlice for [$to; N] {
            #[inline]
            fn from_byte_slice<T: AsRef<[u8]> + ?Sized>(slice: &T) -> Result<&[[$to; N]], Error> {
                let slice = slice.as_ref();
                let len = check_constraints::<[$to; N]>(slice)?;

                // Need to handle the empty case separately as even an empty slices
                // must have a correctly aligned data pointer
                if len == 0 {
                    Ok(&[])
                } else {
                    #[allow(clippy::cast_ptr_alignment)]
                    unsafe {
                        Ok(slice::from_raw_parts(slice.as_ptr() as *const [$to; N], len))
                    }
                }
            }

            #[inline]
            fn from_mut_byte_slice<T: AsMut<[u8]> + ?Sized>(slice: &mut T) -> Result<&mut [[$to; N]], Error> {
                let slice = slice.as_mut();
                let len = check_constraints::<[$to; N]>(slice)?;

                // Need to handle the empty case separately as even an empty slices
                // must have a correctly aligned data pointer
                if len == 0 {
                    Ok(&mut [])
                } else {
                    #[allow(clippy::cast_ptr_alignment)]
                    unsafe {
                        Ok(slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut [$to; N], len))
                    }
                }
            }
        }

        unsafe impl<const N: usize> ToByteSlice for [$to; N] {
            #[inline]
            fn to_byte_slice<T: AsRef<[[$to; N]]> + ?Sized>(slice: &T) -> &[u8] {
                let slice = slice.as_ref();
                let len = slice.len() * mem::size_of::<[$to; N]>();
                unsafe {
                    slice::from_raw_parts(slice.as_ptr() as *const u8, len)
                }
            }
        }

        unsafe impl<const N: usize> ToMutByteSlice for [$to; N] {
            #[inline]
            fn to_mut_byte_slice<T: AsMut<[[$to; N]]> + ?Sized>(slice: &mut T) -> &mut [u8] {
                let slice = slice.as_mut();
                let len = slice.len() * mem::size_of::<[$to; N]>();
                unsafe {
                    slice::from_raw_parts_mut(slice.as_mut_ptr() as *mut u8, len)
                }
            }
        }
    };
);

/// Trait for converting from a byte slice to a slice of a fundamental, built-in numeric type.
///
/// This trait is an implementation detail. Use the [`AsSliceOf`] and [`AsMutSliceOf`] traits.
///
/// [`AsSliceOf`]: trait.AsSliceOf.html
/// [`AsMutSliceOf`]: trait.AsMutSliceOf.html
pub unsafe trait FromByteSlice
where
    Self: Sized,
{
    /// Convert from an immutable byte slice to a immutable slice of a fundamental, built-in
    /// numeric type
    fn from_byte_slice<T: AsRef<[u8]> + ?Sized>(slice: &T) -> Result<&[Self], Error>;
    /// Convert from an mutable byte slice to a mutable slice of a fundamental, built-in numeric
    /// type
    fn from_mut_byte_slice<T: AsMut<[u8]> + ?Sized>(slice: &mut T) -> Result<&mut [Self], Error>;
}

/// Trait for converting from an immutable slice of a fundamental, built-in numeric type to an
/// immutable byte slice.
///
/// This trait is an implementation detail. Use the [`AsByteSlice`] trait.
///
/// [`AsByteSlice`]: trait.AsByteSlice.html
pub unsafe trait ToByteSlice
where
    Self: Sized,
{
    /// Convert from an immutable slice of a fundamental, built-in numeric type to an immutable
    /// byte slice
    fn to_byte_slice<T: AsRef<[Self]> + ?Sized>(slice: &T) -> &[u8];
}

/// Trait for converting from a mutable slice of a fundamental, built-in numeric type to a mutable
/// byte slice.
///
/// This trait is an implementation detail. Use the [`AsMutByteSlice`] trait.
///
/// [`AsMutByteSlice`]: trait.AsMutByteSlice.html
pub unsafe trait ToMutByteSlice
where
    Self: Sized,
{
    /// Convert from a mutable slice of a fundamental, built-in numeric type to a mutable byte
    /// slice
    fn to_mut_byte_slice<T: AsMut<[Self]> + ?Sized>(slice: &mut T) -> &mut [u8];
}

/// Trait for converting from a byte slice to a slice of a fundamental, built-in numeric type.
///
/// # Example
/// ```no_run
/// # extern crate byte_slice_cast;
/// # fn main() {
/// use byte_slice_cast::*;
///
/// let slice = [1u8, 2u8, 3u8, 4u8, 5u8, 6u8];
/// let converted_slice = slice.as_slice_of::<u16>().unwrap();
///
/// if cfg!(target_endian = "big") {
///     assert_eq!(converted_slice, &[0x0102, 0x0304, 0x0506]);
/// } else {
///     assert_eq!(converted_slice, &[0x0201, 0x0403, 0x0605]);
/// }
/// # }
/// ```
pub trait AsSliceOf {
    fn as_slice_of<T: FromByteSlice>(&self) -> Result<&[T], Error>;
}

impl<U: AsRef<[u8]> + ?Sized> AsSliceOf for U {
    #[inline]
    fn as_slice_of<T: FromByteSlice>(&self) -> Result<&[T], Error> {
        FromByteSlice::from_byte_slice(self)
    }
}

/// Trait for converting from a mutable byte slice to a mutable slice of a fundamental, built-in
/// numeric type.
///
/// # Example
/// ```no_run
/// # extern crate byte_slice_cast;
/// # fn main() {
/// use byte_slice_cast::*;
///
/// let mut slice = [1u8, 2u8, 3u8, 4u8, 5u8, 6u8];
/// let converted_slice = slice.as_mut_slice_of::<u16>().unwrap();
///
/// if cfg!(target_endian = "big") {
///     assert_eq!(converted_slice, &[0x0102, 0x0304, 0x0506]);
/// } else {
///     assert_eq!(converted_slice, &[0x0201, 0x0403, 0x0605]);
/// }
/// # }
/// ```
pub trait AsMutSliceOf {
    fn as_mut_slice_of<T: FromByteSlice>(&mut self) -> Result<&mut [T], Error>;
}

impl<U: AsMut<[u8]> + ?Sized> AsMutSliceOf for U {
    #[inline]
    fn as_mut_slice_of<T: FromByteSlice>(&mut self) -> Result<&mut [T], Error> {
        FromByteSlice::from_mut_byte_slice(self)
    }
}

/// Trait for converting from an immutable slice of a fundamental, built-in numeric type to an
/// immutable byte slice.
///
/// # Example
/// ```no_run
/// # extern crate byte_slice_cast;
/// # fn main() {
/// use byte_slice_cast::*;
///
/// let slice: [u16; 3] = [0x0102, 0x0304, 0x0506];
/// let converted_slice = slice.as_byte_slice();
///
/// if cfg!(target_endian = "big") {
///     assert_eq!(converted_slice, &[1u8, 2u8, 3u8, 4u8, 5u8, 6u8]);
/// } else {
///     assert_eq!(converted_slice, &[2u8, 1u8, 4u8, 3u8, 6u8, 5u8]);
/// }
/// # }
/// ```
pub trait AsByteSlice<T> {
    fn as_byte_slice(&self) -> &[u8];
}

impl<T: ToByteSlice, U: AsRef<[T]> + ?Sized> AsByteSlice<T> for U {
    #[inline]
    fn as_byte_slice(&self) -> &[u8] {
        ToByteSlice::to_byte_slice(self)
    }
}

/// Trait for converting from a mutable slice of a fundamental, built-in numeric type to a mutable
/// byte slice.
///
/// # Example
/// ```no_run
/// # extern crate byte_slice_cast;
/// # fn main() {
/// use byte_slice_cast::*;
///
/// let mut slice: [u16; 3] = [0x0102, 0x0304, 0x0506];
/// let converted_slice = slice.as_mut_byte_slice();
///
/// if cfg!(target_endian = "big") {
///     assert_eq!(converted_slice, &mut [1u8, 2u8, 3u8, 4u8, 5u8, 6u8]);
/// } else {
///     assert_eq!(converted_slice, &mut [2u8, 1u8, 4u8, 3u8, 6u8, 5u8]);
/// }
/// # }
/// ```
pub trait AsMutByteSlice<T> {
    fn as_mut_byte_slice(&mut self) -> &mut [u8];
}

impl<T: ToMutByteSlice, U: AsMut<[T]> + ?Sized> AsMutByteSlice<T> for U {
    #[inline]
    fn as_mut_byte_slice(&mut self) -> &mut [u8] {
        ToMutByteSlice::to_mut_byte_slice(self)
    }
}

impl_trait!(u8);
impl_trait!(u16);
impl_trait!(u32);
impl_trait!(u64);
impl_trait!(u128);
impl_trait!(i8);
impl_trait!(i16);
impl_trait!(i32);
impl_trait!(i64);
impl_trait!(i128);
impl_trait!(f32);
impl_trait!(f64);
impl_trait!(usize);
impl_trait!(isize);

impl_trait_array!(u8);
impl_trait_array!(u16);
impl_trait_array!(u32);
impl_trait_array!(u64);
impl_trait_array!(u128);
impl_trait_array!(i8);
impl_trait_array!(i16);
impl_trait_array!(i32);
impl_trait_array!(i64);
impl_trait_array!(i128);
impl_trait_array!(f32);
impl_trait_array!(f64);
impl_trait_array!(usize);
impl_trait_array!(isize);

impl TypeName for () {
    const TYPE_NAME: &'static str = "()";
}

unsafe impl ToByteSlice for () {
    #[inline]
    fn to_byte_slice<T: AsRef<[()]> + ?Sized>(_: &T) -> &[u8] {
        &[]
    }
}

unsafe impl ToMutByteSlice for () {
    #[inline]
    fn to_mut_byte_slice<T: AsMut<[()]> + ?Sized>(_: &mut T) -> &mut [u8] {
        &mut []
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8() {
        let input: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

        let output: &[u8] = input.as_slice_of::<u8>().unwrap();
        assert_eq!(&input, output);

        let output2: &[u8] = input.as_byte_slice();
        assert_eq!(&input, output2);
    }

    #[test]
    fn u16() {
        let slice: [u16; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let bytes = slice.as_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(bytes, &[0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7]);
        } else {
            assert_eq!(bytes, &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0]);
        }

        assert_eq!(
            (bytes[1..]).as_slice_of::<u16>(),
            Err(Error::AlignmentMismatch {
                dst_type: "u16",
                dst_minimum_alignment: mem::align_of::<u16>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_slice_of::<u16>(),
            Err(Error::LengthMismatch {
                dst_type: "u16",
                src_slice_size: 15,
                dst_type_size: 2
            })
        );
        assert_eq!(bytes.as_slice_of::<u16>(), Ok(slice.as_ref()));
    }

    #[cfg(feature = "std")]
    #[test]
    fn u16_error_string() {
        let slice: [u16; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let bytes = slice.as_byte_slice();

        let error = (bytes[1..]).as_slice_of::<u16>().unwrap_err().to_string();
        assert_eq!(
            error,
            "cannot cast a &[u8] into a &[u16]: the slice's address is not divisible by the minimum alignment (2) of u16",
        );
        let error = (bytes[0..15]).as_slice_of::<u16>().unwrap_err().to_string();
        assert_eq!(
            error,
            "cannot cast a &[u8] into a &[u16]: the size (15) of the slice is not divisible by the size (2) of u16"
        );
    }

    #[test]
    fn u32() {
        let slice: [u32; 4] = [0, 1, 2, 3];
        let bytes = slice.as_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(bytes, &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3]);
        } else {
            assert_eq!(bytes, &[0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
        }

        assert_eq!(
            (bytes[1..]).as_slice_of::<u32>(),
            Err(Error::AlignmentMismatch {
                dst_type: "u32",
                dst_minimum_alignment: mem::align_of::<u32>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_slice_of::<u32>(),
            Err(Error::LengthMismatch {
                dst_type: "u32",
                src_slice_size: 15,
                dst_type_size: 4
            })
        );
        assert_eq!(bytes.as_slice_of::<u32>(), Ok(slice.as_ref()));
    }

    #[test]
    fn u64() {
        let slice: [u64; 2] = [0, 1];
        let bytes = slice.as_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(bytes, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        } else {
            assert_eq!(bytes, &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        }

        assert_eq!(
            (bytes[1..]).as_slice_of::<u64>(),
            Err(Error::AlignmentMismatch {
                dst_type: "u64",
                dst_minimum_alignment: mem::align_of::<u64>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_slice_of::<u64>(),
            Err(Error::LengthMismatch {
                dst_type: "u64",
                src_slice_size: 15,
                dst_type_size: 8
            })
        );
        assert_eq!(bytes.as_slice_of::<u64>(), Ok(slice.as_ref()));
    }

    #[test]
    #[allow(clippy::collapsible_if)]
    #[allow(clippy::collapsible_else_if)]
    fn usize() {
        let slice: [usize; 2] = [0, 1];
        let bytes = slice.as_byte_slice();

        if cfg!(target_endian = "big") {
            if cfg!(target_pointer_width = "16") {
                assert_eq!(bytes, &[0, 0, 0, 1]);
            } else if cfg!(target_pointer_width = "32") {
                assert_eq!(bytes, &[0, 0, 0, 0, 0, 0, 0, 1]);
            } else if cfg!(target_pointer_width = "64") {
                assert_eq!(bytes, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
            } else {
                panic!("Unhandled target_endian/target_pointer_width configuration");
            }
        } else {
            if cfg!(target_pointer_width = "16") {
                assert_eq!(bytes, &[0, 0, 1, 0]);
            } else if cfg!(target_pointer_width = "32") {
                assert_eq!(bytes, &[0, 0, 0, 0, 1, 0, 0, 0]);
            } else if cfg!(target_pointer_width = "64") {
                assert_eq!(bytes, &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
            } else {
                panic!("Unhandled target_endian/target_pointer_width configuration");
            }
        }

        assert_eq!(
            (bytes[1..]).as_slice_of::<usize>(),
            Err(Error::AlignmentMismatch {
                dst_type: "usize",
                dst_minimum_alignment: mem::align_of::<usize>()
            })
        );
        assert_eq!(
            (bytes[0..3]).as_slice_of::<usize>(),
            Err(Error::LengthMismatch {
                dst_type: "usize",
                src_slice_size: 3,
                dst_type_size: mem::size_of::<usize>()
            })
        );
        assert_eq!(bytes.as_slice_of::<usize>(), Ok(slice.as_ref()));
    }

    #[test]
    fn f32() {
        let slice: [f32; 4] = [2.0, 1.0, 0.5, 0.25];
        let bytes = slice.as_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(
                bytes,
                [
                    0x40, 0x00, 0x00, 0x00, 0x3f, 0x80, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x00, 0x3e,
                    0x80, 0x00, 0x00
                ]
            );
        } else {
            assert_eq!(
                bytes,
                [
                    0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x3f, 0x00,
                    0x00, 0x80, 0x3e
                ]
            );
        };

        assert_eq!(
            (bytes[1..]).as_slice_of::<f32>(),
            Err(Error::AlignmentMismatch {
                dst_type: "f32",
                dst_minimum_alignment: mem::align_of::<f32>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_slice_of::<f32>(),
            Err(Error::LengthMismatch {
                dst_type: "f32",
                src_slice_size: 15,
                dst_type_size: 4
            })
        );
        assert_eq!(bytes.as_slice_of::<f32>(), Ok(slice.as_ref()));
    }

    #[test]
    fn f64() {
        let slice: [f64; 2] = [2.0, 0.5];
        let bytes = slice.as_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(
                bytes,
                [
                    0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3f, 0xe0, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00
                ]
            );
        } else {
            assert_eq!(
                bytes,
                [
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0xe0, 0x3f
                ]
            );
        };

        assert_eq!(
            (bytes[1..]).as_slice_of::<f64>(),
            Err(Error::AlignmentMismatch {
                dst_type: "f64",
                dst_minimum_alignment: mem::align_of::<f64>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_slice_of::<f64>(),
            Err(Error::LengthMismatch {
                dst_type: "f64",
                src_slice_size: 15,
                dst_type_size: mem::size_of::<f64>()
            })
        );
        assert_eq!(bytes.as_slice_of::<f64>(), Ok(slice.as_ref()));
    }

    #[test]
    fn u16_mut() {
        let mut slice: [u16; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let mut slice_2: [u16; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let bytes = slice_2.as_mut_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(bytes, &[0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7]);
        } else {
            assert_eq!(bytes, &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0]);
        }

        assert_eq!(
            (bytes[1..]).as_mut_slice_of::<u16>(),
            Err(Error::AlignmentMismatch {
                dst_type: "u16",
                dst_minimum_alignment: mem::align_of::<u16>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_mut_slice_of::<u16>(),
            Err(Error::LengthMismatch {
                dst_type: "u16",
                src_slice_size: 15,
                dst_type_size: 2
            })
        );
        assert_eq!(bytes.as_mut_slice_of::<u16>(), Ok(slice.as_mut()));
    }

    #[cfg(feature = "std")]
    #[test]
    fn u16_vec() {
        let vec: Vec<u16> = vec![0, 1, 2, 3, 4, 5, 6, 7];
        let bytes = vec.as_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(bytes, &[0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7]);
        } else {
            assert_eq!(bytes, &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0]);
        }

        assert_eq!(
            (bytes[1..]).as_slice_of::<u16>(),
            Err(Error::AlignmentMismatch {
                dst_type: "u16",
                dst_minimum_alignment: mem::align_of::<u16>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_slice_of::<u16>(),
            Err(Error::LengthMismatch {
                dst_type: "u16",
                src_slice_size: 15,
                dst_type_size: 2
            })
        );
        assert_eq!(bytes.as_slice_of::<u16>(), Ok(vec.as_ref()));
    }

    #[cfg(feature = "std")]
    #[test]
    fn u16_mut_vec() {
        let mut vec: Vec<u16> = vec![0, 1, 2, 3, 4, 5, 6, 7];
        let mut vec_clone = vec.clone();
        let bytes = vec_clone.as_mut_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(bytes, &[0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7]);
        } else {
            assert_eq!(bytes, &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0]);
        }

        assert_eq!(
            (bytes[1..]).as_mut_slice_of::<u16>(),
            Err(Error::AlignmentMismatch {
                dst_type: "u16",
                dst_minimum_alignment: mem::align_of::<u16>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_mut_slice_of::<u16>(),
            Err(Error::LengthMismatch {
                dst_type: "u16",
                src_slice_size: 15,
                dst_type_size: 2
            })
        );
        assert_eq!(bytes.as_mut_slice_of::<u16>(), Ok(vec.as_mut()));
    }

    #[cfg(feature = "std")]
    #[test]
    fn u16_box_slice() {
        let vec: Box<[u16]> = vec![0, 1, 2, 3, 4, 5, 6, 7].into_boxed_slice();
        let bytes = vec.as_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(bytes, &[0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7]);
        } else {
            assert_eq!(bytes, &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0]);
        }

        assert_eq!(
            (bytes[1..]).as_slice_of::<u16>(),
            Err(Error::AlignmentMismatch {
                dst_type: "u16",
                dst_minimum_alignment: mem::align_of::<u16>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_slice_of::<u16>(),
            Err(Error::LengthMismatch {
                dst_type: "u16",
                src_slice_size: 15,
                dst_type_size: 2
            })
        );
        assert_eq!(bytes.as_slice_of::<u16>(), Ok(vec.as_ref()));
    }

    #[cfg(feature = "std")]
    #[test]
    fn u16_mut_box_slice() {
        let mut vec: Box<[u16]> = vec![0, 1, 2, 3, 4, 5, 6, 7].into_boxed_slice();
        let mut vec_clone: Box<[u16]> = vec![0, 1, 2, 3, 4, 5, 6, 7].into_boxed_slice();
        let bytes = vec_clone.as_mut_byte_slice();

        if cfg!(target_endian = "big") {
            assert_eq!(bytes, &[0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7]);
        } else {
            assert_eq!(bytes, &[0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0]);
        }

        assert_eq!(
            (bytes[1..]).as_mut_slice_of::<u16>(),
            Err(Error::AlignmentMismatch {
                dst_type: "u16",
                dst_minimum_alignment: mem::align_of::<u16>()
            })
        );
        assert_eq!(
            (bytes[0..15]).as_mut_slice_of::<u16>(),
            Err(Error::LengthMismatch {
                dst_type: "u16",
                src_slice_size: 15,
                dst_type_size: 2
            })
        );
        assert_eq!(bytes.as_mut_slice_of::<u16>(), Ok(vec.as_mut()));
    }

    #[test]
    fn u16_empty_to_byte_slice() {
        let slice: [u16; 0] = [];
        let bytes = slice.as_byte_slice();

        assert_eq!(bytes, &[]);
    }

    #[test]
    fn u16_empty_from_byte_slice() {
        let bytes: [u8; 0] = [];
        let slice = bytes.as_slice_of::<u16>().unwrap();
        assert_eq!(slice, &[]);
    }

    #[test]
    fn unit() {
        let slice: [(); 4] = [(), (), (), ()];
        let bytes = slice.as_byte_slice();

        assert_eq!(bytes, &[]);
    }

    #[test]
    fn u8_array() {
        let input: &[[u8; 3]] = &[[0, 1, 2], [3, 4, 5], [6, 7, 8], [9, 10, 11], [12, 13, 14]][..];

        let bytes: &[u8] = input.as_byte_slice();
        assert_eq!(bytes, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);

        let output = bytes.as_slice_of::<[u8; 3]>().unwrap();

        assert_eq!(output, input);
    }

    #[test]
    fn u16_array() {
        let input: &[[u16; 3]] = &[[0, 1, 2], [3, 4, 5]][..];

        let bytes: &[u8] = input.as_byte_slice();
        if cfg!(target_endian = "big") {
            assert_eq!(bytes, [0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5]);
        } else {
            assert_eq!(bytes, [0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0]);
        };

        assert_eq!(
            (bytes[1..]).as_slice_of::<[u16; 3]>(),
            Err(Error::AlignmentMismatch {
                dst_type: "[u16 ; N]",
                dst_minimum_alignment: mem::align_of::<[u16; 3]>()
            })
        );
        assert_eq!(
            (bytes[0..4]).as_slice_of::<[u16; 3]>(),
            Err(Error::LengthMismatch {
                dst_type: "[u16 ; N]",
                src_slice_size: 4,
                dst_type_size: 6
            })
        );

        let output = bytes.as_slice_of::<[u16; 3]>().unwrap();
        assert_eq!(output, input);
    }
}
