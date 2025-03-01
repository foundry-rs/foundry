#![allow(unsafe_op_in_unsafe_fn)]

use super::generic;
use crate::get_chars_table;
use core::simd::prelude::*;

type Simd = u8x16;

pub(crate) const USE_CHECK_FN: bool = true;

pub(crate) unsafe fn encode<const UPPER: bool>(input: &[u8], output: *mut u8) {
    // Load table.
    let hex_table = Simd::from_array(*get_chars_table::<UPPER>());

    generic::encode_unaligned_chunks::<UPPER, _>(input, output, |chunk: Simd| {
        // Load input bytes and mask to nibbles.
        let mut lo = chunk & Simd::splat(15);
        let mut hi = chunk >> Simd::splat(4);

        // Lookup the corresponding ASCII hex digit for each nibble.
        lo = hex_table.swizzle_dyn(lo);
        hi = hex_table.swizzle_dyn(hi);

        // Interleave the nibbles ([hi[0], lo[0], hi[1], lo[1], ...]).
        Simd::interleave(hi, lo)
    });
}

pub(crate) fn check(input: &[u8]) -> bool {
    generic::check_unaligned_chunks(input, |chunk: Simd| {
        let valid_digit = chunk.simd_ge(Simd::splat(b'0')) & chunk.simd_le(Simd::splat(b'9'));
        let valid_upper = chunk.simd_ge(Simd::splat(b'A')) & chunk.simd_le(Simd::splat(b'F'));
        let valid_lower = chunk.simd_ge(Simd::splat(b'a')) & chunk.simd_le(Simd::splat(b'f'));
        let valid = valid_digit | valid_upper | valid_lower;
        valid.all()
    })
}

pub(crate) use generic::decode_checked;
pub(crate) use generic::decode_unchecked;
