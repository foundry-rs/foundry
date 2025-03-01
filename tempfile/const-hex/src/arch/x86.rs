#![allow(unsafe_op_in_unsafe_fn)]
#![allow(unexpected_cfgs)]

use super::generic;
use crate::get_chars_table;

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

pub(crate) const USE_CHECK_FN: bool = true;
const CHUNK_SIZE_AVX: usize = core::mem::size_of::<__m256i>();

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        #[inline(always)]
        fn has_sse2() -> bool {
            std::arch::is_x86_feature_detected!("sse2")
        }
        #[inline(always)]
        fn has_ssse3() -> bool {
            std::arch::is_x86_feature_detected!("ssse3")
        }
        #[inline(always)]
        fn has_avx2() -> bool {
            std::arch::is_x86_feature_detected!("avx2")
        }
    } else {
        cpufeatures::new!(cpuid_sse2, "sse2");
        use cpuid_sse2::get as has_sse2;
        cpufeatures::new!(cpuid_ssse3, "ssse3");
        use cpuid_ssse3::get as has_ssse3;
        cpufeatures::new!(cpuid_avx2, "avx2");
        use cpuid_avx2::get as has_avx2;
    }
}

#[inline]
pub(crate) unsafe fn encode<const UPPER: bool>(input: &[u8], output: *mut u8) {
    if !has_ssse3() {
        return generic::encode::<UPPER>(input, output);
    }
    encode_ssse3::<UPPER>(input, output);
}

#[target_feature(enable = "ssse3")]
unsafe fn encode_ssse3<const UPPER: bool>(input: &[u8], output: *mut u8) {
    // Load table.
    let hex_table = _mm_loadu_si128(get_chars_table::<UPPER>().as_ptr().cast());

    generic::encode_unaligned_chunks::<UPPER, _>(input, output, |chunk: __m128i| {
        // Load input bytes and mask to nibbles.
        let mut lo = _mm_and_si128(chunk, _mm_set1_epi8(0x0F));
        #[allow(clippy::cast_possible_wrap)]
        let mut hi = _mm_srli_epi32::<4>(_mm_and_si128(chunk, _mm_set1_epi8(0xF0u8 as i8)));

        // Lookup the corresponding ASCII hex digit for each nibble.
        lo = _mm_shuffle_epi8(hex_table, lo);
        hi = _mm_shuffle_epi8(hex_table, hi);

        // Interleave the nibbles ([hi[0], lo[0], hi[1], lo[1], ...]).
        let hex_lo = _mm_unpacklo_epi8(hi, lo);
        let hex_hi = _mm_unpackhi_epi8(hi, lo);
        (hex_lo, hex_hi)
    });
}

#[inline]
pub(crate) fn check(input: &[u8]) -> bool {
    if !has_sse2() {
        return generic::check(input);
    }
    unsafe { check_sse2(input) }
}

/// Modified from [`faster-hex`](https://github.com/nervosnetwork/faster-hex/blob/856aba7b141a5fe16113fae110d535065882f25a/src/decode.rs).
#[target_feature(enable = "sse2")]
unsafe fn check_sse2(input: &[u8]) -> bool {
    let ascii_zero = _mm_set1_epi8((b'0' - 1) as i8);
    let ascii_nine = _mm_set1_epi8((b'9' + 1) as i8);
    let ascii_ua = _mm_set1_epi8((b'A' - 1) as i8);
    let ascii_uf = _mm_set1_epi8((b'F' + 1) as i8);
    let ascii_la = _mm_set1_epi8((b'a' - 1) as i8);
    let ascii_lf = _mm_set1_epi8((b'f' + 1) as i8);

    generic::check_unaligned_chunks(input, |chunk: __m128i| {
        let ge0 = _mm_cmpgt_epi8(chunk, ascii_zero);
        let le9 = _mm_cmplt_epi8(chunk, ascii_nine);
        let valid_digit = _mm_and_si128(ge0, le9);

        let geua = _mm_cmpgt_epi8(chunk, ascii_ua);
        let leuf = _mm_cmplt_epi8(chunk, ascii_uf);
        let valid_upper = _mm_and_si128(geua, leuf);

        let gela = _mm_cmpgt_epi8(chunk, ascii_la);
        let lelf = _mm_cmplt_epi8(chunk, ascii_lf);
        let valid_lower = _mm_and_si128(gela, lelf);

        let valid_letter = _mm_or_si128(valid_lower, valid_upper);
        let valid_mask = _mm_movemask_epi8(_mm_or_si128(valid_digit, valid_letter));
        valid_mask == 0xffff
    })
}

#[inline]
pub(crate) unsafe fn decode_unchecked(input: &[u8], output: &mut [u8]) {
    if !has_avx2() {
        return generic::decode_unchecked(input, output);
    }
    decode_avx2(input, output);
}

/// Modified from [`faster-hex`](https://github.com/nervosnetwork/faster-hex/blob/856aba7b141a5fe16113fae110d535065882f25a/src/decode.rs).
#[target_feature(enable = "avx2")]
unsafe fn decode_avx2(mut input: &[u8], mut output: &mut [u8]) {
    #[rustfmt::skip]
    let mask_a = _mm256_setr_epi8(
        0, -1, 2, -1, 4, -1, 6, -1, 8, -1, 10, -1, 12, -1, 14, -1,
        0, -1, 2, -1, 4, -1, 6, -1, 8, -1, 10, -1, 12, -1, 14, -1,
    );

    #[rustfmt::skip]
    let mask_b = _mm256_setr_epi8(
        1, -1, 3, -1, 5, -1, 7, -1, 9, -1, 11, -1, 13, -1, 15, -1,
        1, -1, 3, -1, 5, -1, 7, -1, 9, -1, 11, -1, 13, -1, 15, -1
    );

    while output.len() >= CHUNK_SIZE_AVX {
        let av1 = _mm256_loadu_si256(input.as_ptr().cast());
        let av2 = _mm256_loadu_si256(input.as_ptr().add(CHUNK_SIZE_AVX).cast());

        let mut a1 = _mm256_shuffle_epi8(av1, mask_a);
        let mut b1 = _mm256_shuffle_epi8(av1, mask_b);
        let mut a2 = _mm256_shuffle_epi8(av2, mask_a);
        let mut b2 = _mm256_shuffle_epi8(av2, mask_b);

        a1 = unhex_avx2(a1);
        a2 = unhex_avx2(a2);
        b1 = unhex_avx2(b1);
        b2 = unhex_avx2(b2);

        let bytes = nib2byte_avx2(a1, b1, a2, b2);

        // dst does not need to be aligned on any particular boundary
        _mm256_storeu_si256(output.as_mut_ptr() as *mut _, bytes);
        output = output.get_unchecked_mut(32..);
        input = input.get_unchecked(64..);
    }

    generic::decode_unchecked(input, output);
}

#[inline]
#[target_feature(enable = "avx2")]
unsafe fn unhex_avx2(value: __m256i) -> __m256i {
    let sr6 = _mm256_srai_epi16(value, 6);
    let and15 = _mm256_and_si256(value, _mm256_set1_epi16(0xf));
    let mul = _mm256_maddubs_epi16(sr6, _mm256_set1_epi16(9));
    _mm256_add_epi16(mul, and15)
}

// (a << 4) | b;
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn nib2byte_avx2(a1: __m256i, b1: __m256i, a2: __m256i, b2: __m256i) -> __m256i {
    let a4_1 = _mm256_slli_epi16(a1, 4);
    let a4_2 = _mm256_slli_epi16(a2, 4);
    let a4orb_1 = _mm256_or_si256(a4_1, b1);
    let a4orb_2 = _mm256_or_si256(a4_2, b2);
    let pck1 = _mm256_packus_epi16(a4orb_1, a4orb_2);
    _mm256_permute4x64_epi64(pck1, 0b11011000)
}

// Not used.
pub(crate) use generic::decode_checked;
