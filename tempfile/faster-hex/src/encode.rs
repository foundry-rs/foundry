#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[cfg(feature = "alloc")]
use alloc::{string::String, vec};

use crate::error::Error;

static TABLE_LOWER: &[u8] = b"0123456789abcdef";
static TABLE_UPPER: &[u8] = b"0123456789ABCDEF";

#[cfg(any(feature = "alloc", test))]
fn hex_string_custom_case(src: &[u8], upper_case: bool) -> String {
    let mut buffer = vec![0; src.len() * 2];
    if upper_case {
        hex_encode_upper(src, &mut buffer).expect("hex_string");
    } else {
        hex_encode(src, &mut buffer).expect("hex_string");
    }

    if cfg!(debug_assertions) {
        String::from_utf8(buffer).unwrap()
    } else {
        // Saftey: We just wrote valid utf8 hex string into the dst
        unsafe { String::from_utf8_unchecked(buffer) }
    }
}

#[cfg(any(feature = "alloc", test))]
pub fn hex_string(src: &[u8]) -> String {
    hex_string_custom_case(src, false)
}

#[cfg(any(feature = "alloc", test))]
pub fn hex_string_upper(src: &[u8]) -> String {
    hex_string_custom_case(src, true)
}

pub fn hex_encode_custom<'a>(
    src: &[u8],
    dst: &'a mut [u8],
    upper_case: bool,
) -> Result<&'a mut str, Error> {
    unsafe fn mut_str(buffer: &mut [u8]) -> &mut str {
        if cfg!(debug_assertions) {
            core::str::from_utf8_mut(buffer).unwrap()
        } else {
            core::str::from_utf8_unchecked_mut(buffer)
        }
    }

    let expect_dst_len = src
        .len()
        .checked_mul(2)
        .ok_or(Error::InvalidLength(src.len()))?;
    if dst.len() < expect_dst_len {
        return Err(Error::InvalidLength(expect_dst_len));
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        match crate::vectorization_support() {
            crate::Vectorization::AVX2 => unsafe { hex_encode_avx2(src, dst, upper_case) },
            crate::Vectorization::SSE41 => unsafe { hex_encode_sse41(src, dst, upper_case) },
            crate::Vectorization::None => hex_encode_custom_case_fallback(src, dst, upper_case),
        }
        // Safety: We just wrote valid utf8 hex string into the dst
        return Ok(unsafe { mut_str(dst) });
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        hex_encode_custom_case_fallback(src, dst, upper_case);
        // Saftey: We just wrote valid utf8 hex string into the dst
        Ok(unsafe { mut_str(dst) })
    }
}

/// Hex encode src into dst.
/// The length of dst must be at least src.len() * 2.
pub fn hex_encode<'a>(src: &[u8], dst: &'a mut [u8]) -> Result<&'a mut str, Error> {
    hex_encode_custom(src, dst, false)
}

pub fn hex_encode_upper<'a>(src: &[u8], dst: &'a mut [u8]) -> Result<&'a mut str, Error> {
    hex_encode_custom(src, dst, true)
}

#[deprecated(since = "0.3.0", note = "please use `hex_encode` instead")]
pub fn hex_to(src: &[u8], dst: &mut [u8]) -> Result<(), Error> {
    hex_encode(src, dst).map(|_| ())
}

#[target_feature(enable = "avx2")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
unsafe fn hex_encode_avx2(mut src: &[u8], dst: &mut [u8], upper_case: bool) {
    let ascii_zero = _mm256_set1_epi8(b'0' as i8);
    let nines = _mm256_set1_epi8(9);
    let ascii_a = if upper_case {
        _mm256_set1_epi8((b'A' - 9 - 1) as i8)
    } else {
        _mm256_set1_epi8((b'a' - 9 - 1) as i8)
    };
    let and4bits = _mm256_set1_epi8(0xf);

    let mut i = 0_isize;
    while src.len() >= 32 {
        // https://stackoverflow.com/questions/47425851/whats-the-difference-between-mm256-lddqu-si256-and-mm256-loadu-si256
        let invec = _mm256_loadu_si256(src.as_ptr() as *const _);

        let masked1 = _mm256_and_si256(invec, and4bits);
        let masked2 = _mm256_and_si256(_mm256_srli_epi64(invec, 4), and4bits);

        // return 0xff corresponding to the elements > 9, or 0x00 otherwise
        let cmpmask1 = _mm256_cmpgt_epi8(masked1, nines);
        let cmpmask2 = _mm256_cmpgt_epi8(masked2, nines);

        // add '0' or the offset depending on the masks
        let masked1 = _mm256_add_epi8(masked1, _mm256_blendv_epi8(ascii_zero, ascii_a, cmpmask1));
        let masked2 = _mm256_add_epi8(masked2, _mm256_blendv_epi8(ascii_zero, ascii_a, cmpmask2));

        // interleave masked1 and masked2 bytes
        let res1 = _mm256_unpacklo_epi8(masked2, masked1);
        let res2 = _mm256_unpackhi_epi8(masked2, masked1);

        // Store everything into the right destination now
        let base = dst.as_mut_ptr().offset(i * 2);
        let base1 = base.offset(0) as *mut _;
        let base2 = base.offset(16) as *mut _;
        let base3 = base.offset(32) as *mut _;
        let base4 = base.offset(48) as *mut _;
        _mm256_storeu2_m128i(base3, base1, res1);
        _mm256_storeu2_m128i(base4, base2, res2);
        src = &src[32..];
        i += 32;
    }

    let i = i as usize;
    hex_encode_sse41(src, &mut dst[i * 2..], upper_case);
}

// copied from https://github.com/Matherunner/bin2hex-sse/blob/master/base16_sse4.cpp
#[target_feature(enable = "sse4.1")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
unsafe fn hex_encode_sse41(mut src: &[u8], dst: &mut [u8], upper_case: bool) {
    let ascii_zero = _mm_set1_epi8(b'0' as i8);
    let nines = _mm_set1_epi8(9);
    let ascii_a = if upper_case {
        _mm_set1_epi8((b'A' - 9 - 1) as i8)
    } else {
        _mm_set1_epi8((b'a' - 9 - 1) as i8)
    };
    let and4bits = _mm_set1_epi8(0xf);

    let mut i = 0_isize;
    while src.len() >= 16 {
        let invec = _mm_loadu_si128(src.as_ptr() as *const _);

        let masked1 = _mm_and_si128(invec, and4bits);
        let masked2 = _mm_and_si128(_mm_srli_epi64(invec, 4), and4bits);

        // return 0xff corresponding to the elements > 9, or 0x00 otherwise
        let cmpmask1 = _mm_cmpgt_epi8(masked1, nines);
        let cmpmask2 = _mm_cmpgt_epi8(masked2, nines);

        // add '0' or the offset depending on the masks
        let masked1 = _mm_add_epi8(masked1, _mm_blendv_epi8(ascii_zero, ascii_a, cmpmask1));
        let masked2 = _mm_add_epi8(masked2, _mm_blendv_epi8(ascii_zero, ascii_a, cmpmask2));

        // interleave masked1 and masked2 bytes
        let res1 = _mm_unpacklo_epi8(masked2, masked1);
        let res2 = _mm_unpackhi_epi8(masked2, masked1);

        _mm_storeu_si128(dst.as_mut_ptr().offset(i * 2) as *mut _, res1);
        _mm_storeu_si128(dst.as_mut_ptr().offset(i * 2 + 16) as *mut _, res2);
        src = &src[16..];
        i += 16;
    }

    let i = i as usize;
    hex_encode_custom_case_fallback(src, &mut dst[i * 2..], upper_case);
}

#[inline]
fn hex_lower(byte: u8) -> u8 {
    TABLE_LOWER[byte as usize]
}

#[inline]
fn hex_upper(byte: u8) -> u8 {
    TABLE_UPPER[byte as usize]
}

fn hex_encode_custom_case_fallback(src: &[u8], dst: &mut [u8], upper_case: bool) {
    if upper_case {
        for (byte, slots) in src.iter().zip(dst.chunks_exact_mut(2)) {
            slots[0] = hex_upper((*byte >> 4) & 0xf);
            slots[1] = hex_upper(*byte & 0xf);
        }
    } else {
        for (byte, slots) in src.iter().zip(dst.chunks_exact_mut(2)) {
            slots[0] = hex_lower((*byte >> 4) & 0xf);
            slots[1] = hex_lower(*byte & 0xf);
        }
    }
}

pub fn hex_encode_fallback(src: &[u8], dst: &mut [u8]) {
    hex_encode_custom_case_fallback(src, dst, false)
}

pub fn hex_encode_upper_fallback(src: &[u8], dst: &mut [u8]) {
    hex_encode_custom_case_fallback(src, dst, true)
}

#[cfg(test)]
mod tests {
    use crate::encode::{hex_encode, hex_encode_custom_case_fallback};

    use crate::hex_encode_fallback;
    use core::str;
    use proptest::proptest;

    fn _test_encode_fallback(s: &String, upper_case: bool) {
        let mut buffer = vec![0; s.as_bytes().len() * 2];
        hex_encode_custom_case_fallback(s.as_bytes(), &mut buffer, upper_case);

        let encode = unsafe { str::from_utf8_unchecked(&buffer[..s.as_bytes().len() * 2]) };
        if upper_case {
            assert_eq!(encode, hex::encode_upper(s));
        } else {
            assert_eq!(encode, hex::encode(s));
        }
    }

    proptest! {
        #[test]
        fn test_encode_fallback(ref s in ".*") {
            _test_encode_fallback(s, true);
            _test_encode_fallback(s, false);
        }
    }

    #[test]
    fn test_encode_zero_length_src_should_be_ok() {
        let src = b"";
        let mut dst = [0u8; 10];
        assert!(hex_encode(src, &mut dst).is_ok());

        // this function have no return value, so we just execute it and expect no panic
        hex_encode_fallback(src, &mut dst);
    }
}
