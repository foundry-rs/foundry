#![warn(missing_docs)]

use std::iter::FromIterator;

mod internal {
    use crate::{
        decode::{hex_decode_with_case, CheckCase},
        encode::hex_encode_custom,
    };
    use alloc::borrow::Cow;
    use serde::{de::Error, Deserializer, Serializer};
    use std::iter::FromIterator;

    pub(crate) fn serialize<S, T>(
        data: T,
        serializer: S,
        with_prefix: bool,
        case: CheckCase,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: AsRef<[u8]>,
    {
        let src: &[u8] = data.as_ref();

        let mut dst_length = data.as_ref().len() << 1;
        if with_prefix {
            dst_length += 2;
        }

        let mut dst = vec![0u8; dst_length];
        let mut dst_start = 0;
        if with_prefix {
            dst[0] = b'0';
            dst[1] = b'x';

            dst_start = 2;
        }

        hex_encode_custom(src, &mut dst[dst_start..], matches!(case, CheckCase::Upper))
            .map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(unsafe { ::std::str::from_utf8_unchecked(&dst) })
    }

    pub(crate) fn deserialize<'de, D, T>(
        deserializer: D,
        with_prefix: bool,
        check_case: CheckCase,
    ) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        T: FromIterator<u8>,
    {
        let raw_src: Cow<str> = serde::Deserialize::deserialize(deserializer)?;
        if with_prefix && !raw_src.starts_with("0x") {
            return Err(D::Error::custom("invalid prefix".to_string()));
        }

        let src: &[u8] = {
            if with_prefix {
                raw_src[2..].as_bytes()
            } else {
                raw_src.as_bytes()
            }
        };

        if src.len() & 1 != 0 {
            return Err(D::Error::custom("invalid length".to_string()));
        }

        // we have already checked src's length, so src's length is a even integer
        let mut dst = vec![0; src.len() >> 1];
        hex_decode_with_case(src, &mut dst, check_case)
            .map_err(|e| Error::custom(format!("{:?}", e)))?;
        Ok(dst.into_iter().collect())
    }
}

/// Serde: Serialize with 0x-prefix and ignore case
pub fn serialize<S, T>(data: T, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: AsRef<[u8]>,
{
    withpfx_ignorecase::serialize(data, serializer)
}

/// Serde: Deserialize with 0x-prefix and ignore case
pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: FromIterator<u8>,
{
    withpfx_ignorecase::deserialize(deserializer)
}

/// Generate module with serde methods
macro_rules! faster_hex_serde_macros {
    ($mod_name:ident, $with_pfx:expr, $check_case:expr) => {
        /// Serialize and deserialize with or without 0x-prefix,
        /// and lowercase or uppercase or ignorecase
        pub mod $mod_name {
            use crate::decode::CheckCase;
            use crate::serde::internal;
            use std::iter::FromIterator;

            /// Serializes `data` as hex string
            pub fn serialize<S, T>(data: T, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
                T: AsRef<[u8]>,
            {
                internal::serialize(data, serializer, $with_pfx, $check_case)
            }

            /// Deserializes a hex string into raw bytes.
            pub fn deserialize<'de, D, T>(deserializer: D) -> Result<T, D::Error>
            where
                D: serde::Deserializer<'de>,
                T: FromIterator<u8>,
            {
                internal::deserialize(deserializer, $with_pfx, $check_case)
            }
        }
    };
}

// /// Serialize with 0x-prefix and lowercase
// /// When deserialize, expect 0x-prefix and don't care case
faster_hex_serde_macros!(withpfx_ignorecase, true, CheckCase::None);
// /// Serialize without 0x-prefix and lowercase
// /// When deserialize, expect without 0x-prefix and don't care case
faster_hex_serde_macros!(nopfx_ignorecase, false, CheckCase::None);
// /// Serialize with 0x-prefix and lowercase
// /// When deserialize, expect with 0x-prefix and lower case
faster_hex_serde_macros!(withpfx_lowercase, true, CheckCase::Lower);
// /// Serialize without 0x-prefix and lowercase
// /// When deserialize, expect without 0x-prefix and lower case
faster_hex_serde_macros!(nopfx_lowercase, false, CheckCase::Lower);

// /// Serialize with 0x-prefix and upper case
// /// When deserialize, expect with 0x-prefix and upper case
faster_hex_serde_macros!(withpfx_uppercase, true, CheckCase::Upper);
// /// Serialize without 0x-prefix and upper case
// /// When deserialize, expect without 0x-prefix and upper case
faster_hex_serde_macros!(nopfx_uppercase, false, CheckCase::Upper);

#[cfg(test)]
mod tests {
    use super::{
        nopfx_ignorecase, nopfx_lowercase, nopfx_uppercase, withpfx_ignorecase, withpfx_lowercase,
        withpfx_uppercase,
    };
    use crate as faster_hex;
    use bytes::Bytes;
    use proptest::proptest;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Simple {
        #[serde(with = "faster_hex")]
        bar: Vec<u8>,
    }

    #[test]
    fn test_deserialize_escaped() {
        // 0x03 but escaped.
        let x: Simple = serde_json::from_str(
            r#"{
            "bar": "\u0030x\u00303"
        }"#,
        )
        .unwrap();
        assert_eq!(x.bar, b"\x03");
    }

    fn _test_simple(src: &str) {
        let simple = Simple { bar: src.into() };
        let result = serde_json::to_string(&simple);
        assert!(result.is_ok());
        let result = result.unwrap();

        // #[serde(with = "faster_hex")] should result with 0x prefix
        assert!(result.starts_with(r#"{"bar":"0x"#));

        // #[serde(with = "faster_hex")] shouldn't contains uppercase
        assert!(result[7..].chars().all(|c| !c.is_uppercase()));

        let decode_simple = serde_json::from_str::<Simple>(&result);
        assert!(decode_simple.is_ok());
        assert_eq!(decode_simple.unwrap(), simple);
    }

    proptest! {
        #[test]
        fn test_simple(ref s in ".*") {
            _test_simple(s);
        }
    }

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Foo {
        #[serde(with = "nopfx_lowercase")]
        bar_nopfx_lowercase_vec: Vec<u8>,
        #[serde(with = "nopfx_lowercase")]
        bar_nopfx_lowercase_bytes: Bytes,

        #[serde(with = "withpfx_lowercase")]
        bar_withpfx_lowercase_vec: Vec<u8>,
        #[serde(with = "withpfx_lowercase")]
        bar_withpfx_lowercase_bytes: Bytes,

        #[serde(with = "nopfx_uppercase")]
        bar_nopfx_uppercase_vec: Vec<u8>,
        #[serde(with = "nopfx_uppercase")]
        bar_nopfx_uppercase_bytes: Bytes,

        #[serde(with = "withpfx_uppercase")]
        bar_withpfx_uppercase_vec: Vec<u8>,
        #[serde(with = "withpfx_uppercase")]
        bar_withpfx_uppercase_bytes: Bytes,

        #[serde(with = "withpfx_ignorecase")]
        bar_withpfx_ignorecase_vec: Vec<u8>,
        #[serde(with = "withpfx_ignorecase")]
        bar_withpfx_ignorecase_bytes: Bytes,

        #[serde(with = "nopfx_ignorecase")]
        bar_nopfx_ignorecase_vec: Vec<u8>,
        #[serde(with = "nopfx_ignorecase")]
        bar_nopfx_ignorecase_bytes: Bytes,
    }

    #[test]
    fn test_serde_default() {
        {
            let foo_defuault = Foo {
                bar_nopfx_lowercase_vec: vec![],
                bar_nopfx_lowercase_bytes: Default::default(),
                bar_withpfx_lowercase_vec: vec![],
                bar_withpfx_lowercase_bytes: Default::default(),
                bar_nopfx_uppercase_vec: vec![],
                bar_nopfx_uppercase_bytes: Default::default(),
                bar_withpfx_uppercase_vec: vec![],
                bar_withpfx_uppercase_bytes: Default::default(),
                bar_withpfx_ignorecase_vec: vec![],
                bar_withpfx_ignorecase_bytes: Default::default(),
                bar_nopfx_ignorecase_vec: vec![],
                bar_nopfx_ignorecase_bytes: Default::default(),
            };
            let serde_result = serde_json::to_string(&foo_defuault).unwrap();
            let  expect = "{\"bar_nopfx_lowercase_vec\":\"\",\"bar_nopfx_lowercase_bytes\":\"\",\"bar_withpfx_lowercase_vec\":\"0x\",\"bar_withpfx_lowercase_bytes\":\"0x\",\"bar_nopfx_uppercase_vec\":\"\",\"bar_nopfx_uppercase_bytes\":\"\",\"bar_withpfx_uppercase_vec\":\"0x\",\"bar_withpfx_uppercase_bytes\":\"0x\",\"bar_withpfx_ignorecase_vec\":\"0x\",\"bar_withpfx_ignorecase_bytes\":\"0x\",\"bar_nopfx_ignorecase_vec\":\"\",\"bar_nopfx_ignorecase_bytes\":\"\"}";
            assert_eq!(serde_result, expect);

            let foo_src: Foo = serde_json::from_str(&serde_result).unwrap();
            assert_eq!(foo_defuault, foo_src);
        }
    }

    fn _test_serde(src: &str) {
        let foo = Foo {
            bar_nopfx_lowercase_vec: Vec::from(src),
            bar_nopfx_lowercase_bytes: Bytes::from(Vec::from(src)),
            bar_withpfx_lowercase_vec: Vec::from(src),
            bar_withpfx_lowercase_bytes: Bytes::from(Vec::from(src)),
            bar_nopfx_uppercase_vec: Vec::from(src),
            bar_nopfx_uppercase_bytes: Bytes::from(Vec::from(src)),
            bar_withpfx_uppercase_vec: Vec::from(src),
            bar_withpfx_uppercase_bytes: Bytes::from(Vec::from(src)),

            bar_withpfx_ignorecase_vec: Vec::from(src),
            bar_withpfx_ignorecase_bytes: Bytes::from(Vec::from(src)),
            bar_nopfx_ignorecase_vec: Vec::from(src),
            bar_nopfx_ignorecase_bytes: Bytes::from(Vec::from(src)),
        };
        let hex_str = hex::encode(src);
        let hex_str_upper = hex::encode_upper(src);
        let serde_result = serde_json::to_string(&foo).unwrap();

        let  expect = format!("{{\"bar_nopfx_lowercase_vec\":\"{}\",\"bar_nopfx_lowercase_bytes\":\"{}\",\"bar_withpfx_lowercase_vec\":\"0x{}\",\"bar_withpfx_lowercase_bytes\":\"0x{}\",\"bar_nopfx_uppercase_vec\":\"{}\",\"bar_nopfx_uppercase_bytes\":\"{}\",\"bar_withpfx_uppercase_vec\":\"0x{}\",\"bar_withpfx_uppercase_bytes\":\"0x{}\",\"bar_withpfx_ignorecase_vec\":\"0x{}\",\"bar_withpfx_ignorecase_bytes\":\"0x{}\",\"bar_nopfx_ignorecase_vec\":\"{}\",\"bar_nopfx_ignorecase_bytes\":\"{}\"}}",
                              hex_str,
                              hex_str,
                              hex_str,
                              hex_str,
                              hex_str_upper,
                              hex_str_upper,
                              hex_str_upper,
                              hex_str_upper,
                              hex_str,
                              hex_str,
                              hex_str,
                              hex_str,

        );
        assert_eq!(serde_result, expect);

        let foo_src: Foo = serde_json::from_str(&serde_result).unwrap();
        assert_eq!(foo, foo_src);
    }

    proptest! {
        #[test]
        fn test_serde(ref s in ".*") {
            _test_serde(s);
        }
    }

    fn _test_serde_deserialize(src: &str) {
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct FooNoPfxLower {
            #[serde(with = "nopfx_lowercase")]
            bar: Vec<u8>,
        }

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct FooWithPfxLower {
            #[serde(with = "withpfx_lowercase")]
            bar: Vec<u8>,
        }

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct FooNoPfxUpper {
            #[serde(with = "nopfx_uppercase")]
            bar: Vec<u8>,
        }
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct FooWithPfxUpper {
            #[serde(with = "withpfx_uppercase")]
            bar: Vec<u8>,
        }

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct FooNoPfxIgnoreCase {
            #[serde(with = "nopfx_ignorecase")]
            bar: Vec<u8>,
        }
        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct FooWithPfxIgnoreCase {
            #[serde(with = "withpfx_ignorecase")]
            bar: Vec<u8>,
        }

        {
            let hex_foo = serde_json::to_string(&FooNoPfxLower { bar: src.into() }).unwrap();
            let foo_pfx: serde_json::Result<FooWithPfxLower> = serde_json::from_str(&hex_foo);
            // assert foo_pfx is Error, and contains "invalid prefix"
            assert!(foo_pfx.is_err());
            assert!(foo_pfx.unwrap_err().to_string().contains("invalid prefix"));
        }

        {
            let foo_lower = serde_json::to_string(&FooNoPfxLower { bar: src.into() }).unwrap();
            let foo_upper_result: serde_json::Result<FooNoPfxUpper> =
                serde_json::from_str(&foo_lower);
            if hex::encode(src).contains(char::is_lowercase) {
                // FooNoPfxLower's foo field is lowercase, so we can't deserialize it to FooNoPfxUpper
                assert!(foo_upper_result.is_err());
                assert!(foo_upper_result
                    .unwrap_err()
                    .to_string()
                    .contains("Invalid character"));
            }
        }
    }

    proptest! {
        #[test]
        fn test_serde_deserialize(ref s in ".*") {
            _test_serde_deserialize(s);
        }
    }
}
