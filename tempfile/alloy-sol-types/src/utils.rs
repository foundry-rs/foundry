// Copyright 2015-2020 Parity Technologies
// Copyright 2023-2023 Alloy Contributors
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Utilities used by different modules.

use crate::{Error, Result, Word};

const USIZE_BYTES: usize = usize::BITS as usize / 8;

/// Calculates the padded length of a slice by rounding its length to the next
/// word.
#[inline(always)]
pub const fn words_for(data: &[u8]) -> usize {
    words_for_len(data.len())
}

/// Calculates the padded length of a slice of a specific length by rounding its
/// length to the next word.
#[inline(always)]
#[allow(clippy::manual_div_ceil)] // `.div_ceil` has worse codegen: https://godbolt.org/z/MenKWfPh9
pub const fn words_for_len(len: usize) -> usize {
    (len + 31) / 32
}

/// `padded_len` rounds a slice length up to the next multiple of 32
#[inline(always)]
pub(crate) const fn padded_len(data: &[u8]) -> usize {
    next_multiple_of_32(data.len())
}

/// See [`usize::next_multiple_of`].
#[inline(always)]
pub const fn next_multiple_of_32(n: usize) -> usize {
    match n % 32 {
        0 => n,
        r => n + (32 - r),
    }
}

/// Left-pads a `usize` to 32 bytes.
#[inline]
pub(crate) fn pad_usize(value: usize) -> Word {
    let mut padded = Word::ZERO;
    padded[32 - USIZE_BYTES..32].copy_from_slice(&value.to_be_bytes());
    padded
}

/// Returns `Ok(())`. Exists for the [`define_udt!`](crate::define_udt!)'s
/// typecheck.
#[doc(hidden)]
#[inline]
pub const fn just_ok<T>(_: &T) -> crate::Result<()> {
    Ok(())
}

#[inline]
pub(crate) fn check_zeroes(data: &[u8]) -> bool {
    data.iter().all(|b| *b == 0)
}

#[inline]
pub(crate) fn as_offset(word: &Word, validate: bool) -> Result<usize> {
    let (before, data) = word.split_at(32 - USIZE_BYTES);
    if validate && !check_zeroes(before) {
        return Err(Error::type_check_fail(&word[..], "offset (usize)"));
    }
    Ok(usize::from_be_bytes(<[u8; USIZE_BYTES]>::try_from(data).unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::b256;

    #[test]
    fn test_words_for() {
        assert_eq!(words_for(&[]), 0);
        assert_eq!(words_for(&[0; 31]), 1);
        assert_eq!(words_for(&[0; 32]), 1);
        assert_eq!(words_for(&[0; 33]), 2);
    }

    #[test]
    fn test_pad_u32() {
        // this will fail if endianness is not supported
        assert_eq!(
            pad_usize(0),
            b256!("0x0000000000000000000000000000000000000000000000000000000000000000")
        );
        assert_eq!(
            pad_usize(1),
            b256!("0x0000000000000000000000000000000000000000000000000000000000000001")
        );
        assert_eq!(
            pad_usize(0x100),
            b256!("0x0000000000000000000000000000000000000000000000000000000000000100")
        );
        assert_eq!(
            pad_usize(0xffffffff),
            b256!("0x00000000000000000000000000000000000000000000000000000000ffffffff")
        );
    }
}
