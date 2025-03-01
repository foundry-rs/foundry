// avx2 decode modified from https://github.com/zbjornson/fast-hex/blob/master/src/hex.cc

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use crate::error::Error;

const NIL: u8 = u8::MAX;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
const T_MASK: i32 = 65535;

const fn init_unhex_array(check_case: CheckCase) -> [u8; 256] {
    let mut arr = [0; 256];
    let mut i = 0;
    while i < 256 {
        arr[i] = match i as u8 {
            b'0'..=b'9' => i as u8 - b'0',
            b'a'..=b'f' => match check_case {
                CheckCase::Lower | CheckCase::None => i as u8 - b'a' + 10,
                _ => NIL,
            },
            b'A'..=b'F' => match check_case {
                CheckCase::Upper | CheckCase::None => i as u8 - b'A' + 10,
                _ => NIL,
            },
            _ => NIL,
        };
        i += 1;
    }
    arr
}

const fn init_unhex4_array(check_case: CheckCase) -> [u8; 256] {
    let unhex_arr = init_unhex_array(check_case);

    let mut unhex4_arr = [NIL; 256];
    let mut i = 0;
    while i < 256 {
        if unhex_arr[i] != NIL {
            unhex4_arr[i] = unhex_arr[i] << 4;
        }
        i += 1;
    }
    unhex4_arr
}

// ASCII -> hex
pub(crate) static UNHEX: [u8; 256] = init_unhex_array(CheckCase::None);

// ASCII -> hex, lower case
pub(crate) static UNHEX_LOWER: [u8; 256] = init_unhex_array(CheckCase::Lower);

// ASCII -> hex, upper case
pub(crate) static UNHEX_UPPER: [u8; 256] = init_unhex_array(CheckCase::Upper);

// ASCII -> hex << 4
pub(crate) static UNHEX4: [u8; 256] = init_unhex4_array(CheckCase::None);

const _0213: i32 = 0b11011000;

// lower nibble
#[inline]
fn unhex_b(x: usize) -> u8 {
    UNHEX[x]
}

// upper nibble, logically equivalent to unhex_b(x) << 4
#[inline]
fn unhex_a(x: usize) -> u8 {
    UNHEX4[x]
}

#[inline]
#[target_feature(enable = "avx2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
unsafe fn unhex_avx2(value: __m256i) -> __m256i {
    let sr6 = _mm256_srai_epi16(value, 6);
    let and15 = _mm256_and_si256(value, _mm256_set1_epi16(0xf));
    let mul = _mm256_maddubs_epi16(sr6, _mm256_set1_epi16(9));
    _mm256_add_epi16(mul, and15)
}

// (a << 4) | b;
#[inline]
#[target_feature(enable = "avx2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
unsafe fn nib2byte_avx2(a1: __m256i, b1: __m256i, a2: __m256i, b2: __m256i) -> __m256i {
    let a4_1 = _mm256_slli_epi16(a1, 4);
    let a4_2 = _mm256_slli_epi16(a2, 4);
    let a4orb_1 = _mm256_or_si256(a4_1, b1);
    let a4orb_2 = _mm256_or_si256(a4_2, b2);
    let pck1 = _mm256_packus_epi16(a4orb_1, a4orb_2);
    _mm256_permute4x64_epi64(pck1, _0213)
}

/// Check if the input is valid hex bytes slice
pub fn hex_check(src: &[u8]) -> bool {
    hex_check_with_case(src, CheckCase::None)
}

/// Check if the input is valid hex bytes slice with case check
pub fn hex_check_with_case(src: &[u8], check_case: CheckCase) -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        match crate::vectorization_support() {
            crate::Vectorization::AVX2 | crate::Vectorization::SSE41 => unsafe {
                hex_check_sse_with_case(src, check_case)
            },
            crate::Vectorization::None => hex_check_fallback_with_case(src, check_case),
        }
    }

    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    hex_check_fallback_with_case(src, check_case)
}

/// Check if the input is valid hex bytes slice
pub fn hex_check_fallback(src: &[u8]) -> bool {
    hex_check_fallback_with_case(src, CheckCase::None)
}

/// Check if the input is valid hex bytes slice with case check
pub fn hex_check_fallback_with_case(src: &[u8], check_case: CheckCase) -> bool {
    match check_case {
        CheckCase::None => src.iter().all(|&x| UNHEX[x as usize] != NIL),
        CheckCase::Lower => src.iter().all(|&x| UNHEX_LOWER[x as usize] != NIL),
        CheckCase::Upper => src.iter().all(|&x| UNHEX_UPPER[x as usize] != NIL),
    }
}

/// # Safety
/// Check if a byte slice is valid.
#[target_feature(enable = "sse4.1")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub unsafe fn hex_check_sse(src: &[u8]) -> bool {
    hex_check_sse_with_case(src, CheckCase::None)
}

#[derive(Eq, PartialEq)]
pub enum CheckCase {
    None,
    Lower,
    Upper,
}

/// # Safety
/// Check if a byte slice is valid on given check_case.
#[target_feature(enable = "sse4.1")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub unsafe fn hex_check_sse_with_case(mut src: &[u8], check_case: CheckCase) -> bool {
    let ascii_zero = _mm_set1_epi8((b'0' - 1) as i8);
    let ascii_nine = _mm_set1_epi8((b'9' + 1) as i8);
    let ascii_ua = _mm_set1_epi8((b'A' - 1) as i8);
    let ascii_uf = _mm_set1_epi8((b'F' + 1) as i8);
    let ascii_la = _mm_set1_epi8((b'a' - 1) as i8);
    let ascii_lf = _mm_set1_epi8((b'f' + 1) as i8);

    while src.len() >= 16 {
        let unchecked = _mm_loadu_si128(src.as_ptr() as *const _);

        let gt0 = _mm_cmpgt_epi8(unchecked, ascii_zero);
        let lt9 = _mm_cmplt_epi8(unchecked, ascii_nine);
        let valid_digit = _mm_and_si128(gt0, lt9);

        let (valid_la_lf, valid_ua_uf) = match check_case {
            CheckCase::None => {
                let gtua = _mm_cmpgt_epi8(unchecked, ascii_ua);
                let ltuf = _mm_cmplt_epi8(unchecked, ascii_uf);

                let gtla = _mm_cmpgt_epi8(unchecked, ascii_la);
                let ltlf = _mm_cmplt_epi8(unchecked, ascii_lf);

                (
                    Some(_mm_and_si128(gtla, ltlf)),
                    Some(_mm_and_si128(gtua, ltuf)),
                )
            }
            CheckCase::Lower => {
                let gtla = _mm_cmpgt_epi8(unchecked, ascii_la);
                let ltlf = _mm_cmplt_epi8(unchecked, ascii_lf);

                (Some(_mm_and_si128(gtla, ltlf)), None)
            }
            CheckCase::Upper => {
                let gtua = _mm_cmpgt_epi8(unchecked, ascii_ua);
                let ltuf = _mm_cmplt_epi8(unchecked, ascii_uf);
                (None, Some(_mm_and_si128(gtua, ltuf)))
            }
        };

        let valid_letter = match (valid_la_lf, valid_ua_uf) {
            (Some(valid_lower), Some(valid_upper)) => _mm_or_si128(valid_lower, valid_upper),
            (Some(valid_lower), None) => valid_lower,
            (None, Some(valid_upper)) => valid_upper,
            _ => unreachable!(),
        };

        let ret = _mm_movemask_epi8(_mm_or_si128(valid_digit, valid_letter));

        if ret != T_MASK {
            return false;
        }

        src = &src[16..];
    }
    hex_check_fallback_with_case(src, check_case)
}

/// Hex decode src into dst.
/// The length of src must be even, and it's allowed to decode a zero length src.
/// The length of dst must be src.len() / 2.
pub fn hex_decode(src: &[u8], dst: &mut [u8]) -> Result<(), Error> {
    hex_decode_with_case(src, dst, CheckCase::None)
}

/// Hex decode src into dst.
/// The length of src must be even, and it's allowed to decode a zero length src.
/// The length of dst must be src.len() / 2.
/// when check_case is CheckCase::Lower, the hex string must be lower case.
/// when check_case is CheckCase::Upper, the hex string must be upper case.
/// when check_case is CheckCase::None, the hex string can be lower case or upper case.
pub fn hex_decode_with_case(
    src: &[u8],
    dst: &mut [u8],
    check_case: CheckCase,
) -> Result<(), Error> {
    let len = dst.len().checked_mul(2).ok_or(Error::Overflow)?;
    if src.len() < len || ((src.len() & 1) != 0) {
        return Err(Error::InvalidLength(len));
    }

    if !hex_check_with_case(src, check_case) {
        return Err(Error::InvalidChar);
    }
    hex_decode_unchecked(src, dst);
    Ok(())
}

pub fn hex_decode_unchecked(src: &[u8], dst: &mut [u8]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        match crate::vectorization_support() {
            crate::Vectorization::AVX2 => unsafe { hex_decode_avx2(src, dst) },
            crate::Vectorization::None | crate::Vectorization::SSE41 => {
                hex_decode_fallback(src, dst)
            }
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    hex_decode_fallback(src, dst);
}

#[target_feature(enable = "avx2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
unsafe fn hex_decode_avx2(mut src: &[u8], mut dst: &mut [u8]) {
    // 0, -1, 2, -1, 4, -1, 6, -1, 8, -1, 10, -1, 12, -1, 14, -1,
    // 0, -1, 2, -1, 4, -1, 6, -1, 8, -1, 10, -1, 12, -1, 14, -1
    let mask_a = _mm256_setr_epi8(
        0, -1, 2, -1, 4, -1, 6, -1, 8, -1, 10, -1, 12, -1, 14, -1, 0, -1, 2, -1, 4, -1, 6, -1, 8,
        -1, 10, -1, 12, -1, 14, -1,
    );

    // 1, -1, 3, -1, 5, -1, 7, -1, 9, -1, 11, -1, 13, -1, 15, -1,
    // 1, -1, 3, -1, 5, -1, 7, -1, 9, -1, 11, -1, 13, -1, 15, -1
    let mask_b = _mm256_setr_epi8(
        1, -1, 3, -1, 5, -1, 7, -1, 9, -1, 11, -1, 13, -1, 15, -1, 1, -1, 3, -1, 5, -1, 7, -1, 9,
        -1, 11, -1, 13, -1, 15, -1,
    );

    while dst.len() >= 32 {
        let av1 = _mm256_loadu_si256(src.as_ptr() as *const _);
        let av2 = _mm256_loadu_si256(src[32..].as_ptr() as *const _);

        let mut a1 = _mm256_shuffle_epi8(av1, mask_a);
        let mut b1 = _mm256_shuffle_epi8(av1, mask_b);
        let mut a2 = _mm256_shuffle_epi8(av2, mask_a);
        let mut b2 = _mm256_shuffle_epi8(av2, mask_b);

        a1 = unhex_avx2(a1);
        a2 = unhex_avx2(a2);
        b1 = unhex_avx2(b1);
        b2 = unhex_avx2(b2);

        let bytes = nib2byte_avx2(a1, b1, a2, b2);

        //dst does not need to be aligned on any particular boundary
        _mm256_storeu_si256(dst.as_mut_ptr() as *mut _, bytes);
        dst = &mut dst[32..];
        src = &src[64..];
    }
    hex_decode_fallback(src, dst)
}

pub fn hex_decode_fallback(src: &[u8], dst: &mut [u8]) {
    for (slot, bytes) in dst.iter_mut().zip(src.chunks_exact(2)) {
        let a = unhex_a(bytes[0] as usize);
        let b = unhex_b(bytes[1] as usize);
        *slot = a | b;
    }
}

#[cfg(test)]
mod tests {
    use crate::decode::NIL;
    use crate::{
        decode::{
            hex_check_fallback, hex_check_fallback_with_case, hex_decode_fallback, CheckCase,
        },
        encode::hex_string,
    };
    use proptest::proptest;

    fn _test_decode_fallback(s: &String) {
        let len = s.as_bytes().len();
        let mut dst = Vec::with_capacity(len);
        dst.resize(len, 0);

        let hex_string = hex_string(s.as_bytes());

        hex_decode_fallback(hex_string.as_bytes(), &mut dst);

        assert_eq!(&dst[..], s.as_bytes());
    }

    proptest! {
        #[test]
        fn test_decode_fallback(ref s in ".+") {
            _test_decode_fallback(s);
        }
    }

    fn _test_check_fallback_true(s: &String) {
        assert!(hex_check_fallback(s.as_bytes()));
        match (
            s.contains(char::is_lowercase),
            s.contains(char::is_uppercase),
        ) {
            (true, true) => {
                assert!(!hex_check_fallback_with_case(
                    s.as_bytes(),
                    CheckCase::Lower
                ));
                assert!(!hex_check_fallback_with_case(
                    s.as_bytes(),
                    CheckCase::Upper
                ));
            }
            (true, false) => {
                assert!(hex_check_fallback_with_case(s.as_bytes(), CheckCase::Lower));
                assert!(!hex_check_fallback_with_case(
                    s.as_bytes(),
                    CheckCase::Upper
                ));
            }
            (false, true) => {
                assert!(!hex_check_fallback_with_case(
                    s.as_bytes(),
                    CheckCase::Lower
                ));
                assert!(hex_check_fallback_with_case(s.as_bytes(), CheckCase::Upper));
            }
            (false, false) => {
                assert!(hex_check_fallback_with_case(s.as_bytes(), CheckCase::Lower));
                assert!(hex_check_fallback_with_case(s.as_bytes(), CheckCase::Upper));
            }
        }
    }

    proptest! {
    #[test]
        fn test_check_fallback_true(ref s in "[0-9a-fA-F]+") {
            _test_check_fallback_true(s);
        }
    }

    fn _test_check_fallback_false(s: &String) {
        assert!(!hex_check_fallback(s.as_bytes()));
        assert!(!hex_check_fallback_with_case(
            s.as_bytes(),
            CheckCase::Upper
        ));
        assert!(!hex_check_fallback_with_case(
            s.as_bytes(),
            CheckCase::Lower
        ));
    }

    proptest! {
        #[test]
        fn test_check_fallback_false(ref s in ".{16}[^0-9a-fA-F]+") {
            _test_check_fallback_false(s);
        }
    }

    #[test]
    fn test_init_static_array_is_right() {
        static OLD_UNHEX: [u8; 256] = [
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, 0, 1, 2, 3, 4, 5,
            6, 7, 8, 9, NIL, NIL, NIL, NIL, NIL, NIL, NIL, 10, 11, 12, 13, 14, 15, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, 10, 11, 12, 13, 14, 15, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
        ];

        static OLD_UNHEX4: [u8; 256] = [
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, 0, 16, 32, 48,
            64, 80, 96, 112, 128, 144, NIL, NIL, NIL, NIL, NIL, NIL, NIL, 160, 176, 192, 208, 224,
            240, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, 160, 176, 192, 208, 224, 240, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
            NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL, NIL,
        ];

        assert_eq!(OLD_UNHEX, crate::decode::UNHEX);
        assert_eq!(OLD_UNHEX4, crate::decode::UNHEX4);
    }
}

#[cfg(all(test, any(target_arch = "x86", target_arch = "x86_64")))]
mod test_sse {
    use crate::decode::{
        hex_check, hex_check_fallback, hex_check_fallback_with_case, hex_check_sse,
        hex_check_sse_with_case, hex_check_with_case, hex_decode, hex_decode_unchecked,
        hex_decode_with_case, CheckCase,
    };
    use proptest::proptest;

    fn _test_check_sse_with_case(s: &String, check_case: CheckCase, expect_result: bool) {
        if is_x86_feature_detected!("sse4.1") {
            assert_eq!(
                unsafe { hex_check_sse_with_case(s.as_bytes(), check_case) },
                expect_result
            )
        }
    }

    fn _test_check_sse_true(s: &String) {
        if is_x86_feature_detected!("sse4.1") {
            assert!(unsafe { hex_check_sse(s.as_bytes()) });
        }
    }

    proptest! {
    #[test]
    fn test_check_sse_true(ref s in "([0-9a-fA-F][0-9a-fA-F])+") {
            _test_check_sse_true(s);
            _test_check_sse_with_case(s, CheckCase::None, true);
            match (s.contains(char::is_lowercase), s.contains(char::is_uppercase)){
                (true, true) => {
                    _test_check_sse_with_case(s, CheckCase::Lower, false);
                    _test_check_sse_with_case(s, CheckCase::Upper, false);
                },
                (true, false) => {
                    _test_check_sse_with_case(s, CheckCase::Lower, true);
                    _test_check_sse_with_case(s, CheckCase::Upper, false);
                },
                (false, true) => {
                    _test_check_sse_with_case(s, CheckCase::Lower, false);
                    _test_check_sse_with_case(s, CheckCase::Upper, true);
                },
                (false, false) => {
                    _test_check_sse_with_case(s, CheckCase::Lower, true);
                    _test_check_sse_with_case(s, CheckCase::Upper, true);
                }
            }
        }
    }

    fn _test_check_sse_false(s: &String) {
        if is_x86_feature_detected!("sse4.1") {
            assert!(!unsafe { hex_check_sse(s.as_bytes()) });
        }
    }

    proptest! {
        #[test]
        fn test_check_sse_false(ref s in ".{16}[^0-9a-fA-F]+") {
            _test_check_sse_false(s);
            _test_check_sse_with_case(s, CheckCase::None, false);
            _test_check_sse_with_case(s, CheckCase::Lower, false);
            _test_check_sse_with_case(s, CheckCase::Upper, false);
        }
    }

    #[test]
    fn test_decode_zero_length_src_should_not_be_ok() {
        let src = b"";
        let mut dst = [0u8; 10];
        assert!(
            matches!(hex_decode(src, &mut dst), Err(crate::Error::InvalidLength(len)) if len == 20)
        );
        assert!(
            matches!(hex_decode_with_case(src, &mut dst, CheckCase::None), Err(crate::Error::InvalidLength(len)) if len == 20)
        );
        assert!(hex_check(src));
        assert!(hex_check_with_case(src, CheckCase::None));
        assert!(hex_check_fallback(src));
        assert!(hex_check_fallback_with_case(src, CheckCase::None));

        if is_x86_feature_detected!("sse4.1") {
            assert!(unsafe { hex_check_sse_with_case(src, CheckCase::None) });
            assert!(unsafe { hex_check_sse(src) });
        }

        // this function have no return value, so we just execute it and expect no panic
        hex_decode_unchecked(src, &mut dst);
    }

    // If `dst's length` is greater than `src's length * 2`, `hex_decode` should return error
    #[test]
    fn test_if_dst_len_gt_expect_len_should_return_error() {
        let short_str = b"8e40af02265360d59f4ecf9ae9ebf8f00a3118408f5a9cdcbcc9c0f93642f3"; // 62 bytes
        {
            let mut dst = [0u8; 31];
            let result = hex_decode(short_str.as_slice(), &mut dst);
            assert!(result.is_ok());
        }

        {
            let mut dst = [0u8; 32];
            let result = hex_decode(short_str.as_slice(), &mut dst);
            assert!(matches!(result, Err(crate::Error::InvalidLength(len)) if len == 64))
        }

        {
            let mut dst = [0u8; 33];
            let result = hex_decode(short_str.as_slice(), &mut dst);
            assert!(matches!(result, Err(crate::Error::InvalidLength(len)) if len == 66))
        }
    }

    // if both `src` and `dst` are empty, it's ok
    // if `src` is empty, but `dst` is not empty, it should be reported as error
    #[test]
    fn test_decode_zero_src() {
        let zero_src = b"";
        {
            let mut zero_dst = [];
            assert!(hex_decode(zero_src, &mut zero_dst).is_ok());
        }

        {
            let mut non_zero_dst = [0u8; 1];
            assert!(
                matches!(hex_decode(zero_src, &mut non_zero_dst), Err(crate::Error::InvalidLength(len)) if len == 2)
            );
        }
    }
}
