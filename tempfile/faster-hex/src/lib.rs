#![cfg_attr(not(any(test, feature = "std")), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod decode;
mod encode;
mod error;

#[cfg(feature = "serde")]
mod serde;

pub use crate::decode::{
    hex_check, hex_check_fallback, hex_check_with_case, hex_decode, hex_decode_fallback,
    hex_decode_unchecked,
};
pub use crate::encode::{
    hex_encode, hex_encode_fallback, hex_encode_upper, hex_encode_upper_fallback,
};
#[cfg(feature = "alloc")]
pub use crate::encode::{hex_string, hex_string_upper};

pub use crate::error::Error;

#[cfg(feature = "serde")]
pub use crate::serde::{
    deserialize, nopfx_ignorecase, nopfx_lowercase, nopfx_uppercase, serialize, withpfx_ignorecase,
    withpfx_lowercase, withpfx_uppercase,
};

#[allow(deprecated)]
pub use crate::encode::hex_to;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub use crate::decode::{hex_check_sse, hex_check_sse_with_case};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum Vectorization {
    None = 0,
    SSE41 = 1,
    AVX2 = 2,
}

#[inline(always)]
pub(crate) fn vectorization_support() -> Vectorization {
    #[cfg(all(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        use core::sync::atomic::{AtomicU8, Ordering};
        static FLAGS: AtomicU8 = AtomicU8::new(u8::MAX);

        // We're OK with relaxed, worst case scenario multiple threads checked the CPUID.
        let current_flags = FLAGS.load(Ordering::Relaxed);
        // u8::MAX means uninitialized.
        if current_flags != u8::MAX {
            return match current_flags {
                0 => Vectorization::None,
                1 => Vectorization::SSE41,
                2 => Vectorization::AVX2,
                _ => unreachable!(),
            };
        }

        let val = vectorization_support_no_cache_x86();

        FLAGS.store(val as u8, Ordering::Relaxed);
        return val;
    }
    #[allow(unreachable_code)]
    Vectorization::None
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[cold]
fn vectorization_support_no_cache_x86() -> Vectorization {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::__cpuid_count;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::__cpuid_count;

    // SGX doesn't support CPUID,
    // If there's no SSE there might not be CPUID and there's no SSE4.1/AVX2
    if cfg!(target_env = "sgx") || !cfg!(target_feature = "sse") {
        return Vectorization::None;
    }

    let proc_info_ecx = unsafe { __cpuid_count(1, 0) }.ecx;
    let have_sse4 = (proc_info_ecx >> 19) & 1 == 1;
    // If there's no SSE4 there can't be AVX2.
    if !have_sse4 {
        return Vectorization::None;
    }

    let have_xsave = (proc_info_ecx >> 26) & 1 == 1;
    let have_osxsave = (proc_info_ecx >> 27) & 1 == 1;
    let have_avx = (proc_info_ecx >> 27) & 1 == 1;
    if have_xsave && have_osxsave && have_avx {
        // # Safety: We checked that the processor supports xsave
        if unsafe { avx2_support_no_cache_x86() } {
            return Vectorization::AVX2;
        }
    }
    Vectorization::SSE41
}

// We enable xsave so it can inline the _xgetbv call.
// # Safety: Safe as long it's only called when xsave is supported
#[target_feature(enable = "xsave")]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[cold]
unsafe fn avx2_support_no_cache_x86() -> bool {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{__cpuid_count, _xgetbv};
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{__cpuid_count, _xgetbv};

    let xcr0 = _xgetbv(0);
    let os_avx_support = xcr0 & 6 == 6;
    if os_avx_support {
        let extended_features_ebx = __cpuid_count(7, 0).ebx;
        let have_avx2 = (extended_features_ebx >> 5) & 1 == 1;
        if have_avx2 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use crate::decode::{hex_decode, hex_decode_with_case, CheckCase};
    use crate::encode::{hex_encode, hex_string};
    use crate::{hex_encode_upper, hex_string_upper, vectorization_support, Vectorization};
    use proptest::proptest;

    #[test]
    fn test_feature_detection() {
        let vector_support = vectorization_support();
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            match vector_support {
                Vectorization::AVX2 => assert!(is_x86_feature_detected!("avx2")),
                Vectorization::SSE41 => assert!(is_x86_feature_detected!("sse4.1")),
                Vectorization::None => assert!(
                    !cfg!(target_feature = "sse")
                        || !is_x86_feature_detected!("avx2") && !is_x86_feature_detected!("sse4.1")
                ),
            }
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        assert_eq!(vector_support, Vectorization::None);
    }

    fn _test_hex_encode(s: &String) {
        let mut buffer = vec![0; s.as_bytes().len() * 2];
        {
            let encode = &*hex_encode(s.as_bytes(), &mut buffer).unwrap();

            let hex_string = hex_string(s.as_bytes());

            assert_eq!(encode, hex::encode(s));
            assert_eq!(hex_string, hex::encode(s));
        }

        {
            let encode_upper = &*hex_encode_upper(s.as_bytes(), &mut buffer).unwrap();

            let hex_string_upper = hex_string_upper(s.as_bytes());

            assert_eq!(encode_upper, hex::encode_upper(s));
            assert_eq!(hex_string_upper, hex::encode_upper(s));
        }
    }

    proptest! {
        #[test]
        fn test_hex_encode(ref s in ".*") {
            _test_hex_encode(s);
        }
    }

    fn _test_hex_decode(s: &String) {
        let len = s.as_bytes().len();

        {
            let mut dst = Vec::with_capacity(len);
            dst.resize(len, 0);
            let hex_string = hex_string(s.as_bytes());

            hex_decode(hex_string.as_bytes(), &mut dst).unwrap();

            hex_decode_with_case(hex_string.as_bytes(), &mut dst, CheckCase::Lower).unwrap();

            assert_eq!(&dst[..], s.as_bytes());
        }
        {
            let mut dst = Vec::with_capacity(len);
            dst.resize(len, 0);
            let hex_string_upper = hex_string_upper(s.as_bytes());

            hex_decode_with_case(hex_string_upper.as_bytes(), &mut dst, CheckCase::Upper).unwrap();

            assert_eq!(&dst[..], s.as_bytes());
        }
    }

    proptest! {
        #[test]
        fn test_hex_decode(ref s in ".+") {
            _test_hex_decode(s);
        }
    }

    fn _test_hex_decode_check(s: &String, ok: bool) {
        let len = s.as_bytes().len();
        let mut dst = Vec::with_capacity(len / 2);
        dst.resize(len / 2, 0);
        assert!(hex_decode(s.as_bytes(), &mut dst).is_ok() == ok);
    }

    proptest! {
        #[test]
        fn test_hex_decode_check(ref s in "([0-9a-fA-F][0-9a-fA-F])+") {
            _test_hex_decode_check(s, true);
        }
    }

    proptest! {
        #[test]
        fn test_hex_decode_check_odd(ref s in "[0-9a-fA-F]{11}") {
            _test_hex_decode_check(s, false);
        }
    }
}
