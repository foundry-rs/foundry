#![allow(unsafe_op_in_unsafe_fn)]

use super::generic;
use crate::get_chars_table;
use core::arch::aarch64::*;

pub(crate) const USE_CHECK_FN: bool = true;

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        #[inline(always)]
        fn has_neon() -> bool {
            std::arch::is_aarch64_feature_detected!("neon")
        }
    } else {
        #[inline(always)]
        fn has_neon() -> bool {
            cfg!(target_feature = "neon")
        }
    }
}

#[inline]
pub(crate) unsafe fn encode<const UPPER: bool>(input: &[u8], output: *mut u8) {
    if cfg!(miri) || !has_neon() {
        return generic::encode::<UPPER>(input, output);
    }
    encode_neon::<UPPER>(input, output);
}

#[target_feature(enable = "neon")]
pub(crate) unsafe fn encode_neon<const UPPER: bool>(input: &[u8], output: *mut u8) {
    // Load table.
    let hex_table = vld1q_u8(get_chars_table::<UPPER>().as_ptr());

    generic::encode_unaligned_chunks::<UPPER, _>(input, output, |chunk: uint8x16_t| {
        // Load input bytes and mask to nibbles.
        let mut lo = vandq_u8(chunk, vdupq_n_u8(0x0F));
        let mut hi = vshrq_n_u8(chunk, 4);

        // Lookup the corresponding ASCII hex digit for each nibble.
        lo = vqtbl1q_u8(hex_table, lo);
        hi = vqtbl1q_u8(hex_table, hi);

        // Interleave the nibbles ([hi[0], lo[0], hi[1], lo[1], ...]).
        let hex_lo = vzip1q_u8(hi, lo);
        let hex_hi = vzip2q_u8(hi, lo);
        (hex_lo, hex_hi)
    });
}

#[inline]
pub(crate) fn check(input: &[u8]) -> bool {
    if cfg!(miri) || !has_neon() {
        return generic::check(input);
    }
    unsafe { check_neon(input) }
}

#[target_feature(enable = "neon")]
pub(crate) unsafe fn check_neon(input: &[u8]) -> bool {
    generic::check_unaligned_chunks(input, |chunk: uint8x16_t| {
        let ge0 = vcgeq_u8(chunk, vdupq_n_u8(b'0'));
        let le9 = vcleq_u8(chunk, vdupq_n_u8(b'9'));
        let valid_digit = vandq_u8(ge0, le9);

        let geua = vcgeq_u8(chunk, vdupq_n_u8(b'A'));
        let leuf = vcleq_u8(chunk, vdupq_n_u8(b'F'));
        let valid_upper = vandq_u8(geua, leuf);

        let gela = vcgeq_u8(chunk, vdupq_n_u8(b'a'));
        let lelf = vcleq_u8(chunk, vdupq_n_u8(b'f'));
        let valid_lower = vandq_u8(gela, lelf);

        let valid_letter = vorrq_u8(valid_lower, valid_upper);
        let valid_mask = vorrq_u8(valid_digit, valid_letter);
        vminvq_u8(valid_mask) == 0xFF
    })
}

pub(crate) use generic::decode_checked;
pub(crate) use generic::decode_unchecked;
