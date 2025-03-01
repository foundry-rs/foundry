//! Hex literal macro implementation.
//!
//! Modified from the [`hex-literal`](https://github.com/RustCrypto/utils/tree/master/hex-literal)
//! crate to allow `0x` prefixes.

const fn next_hex_char(string: &[u8], mut pos: usize) -> Option<(u8, usize)> {
    while pos < string.len() {
        let raw_val = string[pos];
        pos += 1;
        let val = match raw_val {
            b'0'..=b'9' => raw_val - 48,
            b'A'..=b'F' => raw_val - 55,
            b'a'..=b'f' => raw_val - 87,
            b' ' | b'\r' | b'\n' | b'\t' => continue,
            0..=127 => panic!("Encountered invalid ASCII character"),
            _ => panic!("Encountered non-ASCII character"),
        };
        return Some((val, pos));
    }
    None
}

const fn next_byte(string: &[u8], pos: usize) -> Option<(u8, usize)> {
    let (half1, pos) = match next_hex_char(string, pos) {
        Some(v) => v,
        None => return None,
    };
    let (half2, pos) = match next_hex_char(string, pos) {
        Some(v) => v,
        None => panic!("Odd number of hex characters"),
    };
    Some(((half1 << 4) + half2, pos))
}

/// Strips the `0x` prefix from a hex string.
///
/// This function is an implementation detail and SHOULD NOT be called directly!
#[doc(hidden)]
pub const fn strip_hex_prefix(string: &[u8]) -> &[u8] {
    if let [b'0', b'x' | b'X', rest @ ..] = string {
        rest
    } else {
        string
    }
}

/// Compute length of a byte array which will be decoded from the strings.
///
/// This function is an implementation detail and SHOULD NOT be called directly!
#[doc(hidden)]
pub const fn len(strings: &[&[u8]]) -> usize {
    let mut i = 0;
    let mut len = 0;
    while i < strings.len() {
        let mut pos = 0;
        while let Some((_, new_pos)) = next_byte(strings[i], pos) {
            len += 1;
            pos = new_pos;
        }
        i += 1;
    }
    len
}

/// Decode hex strings into a byte array of pre-computed length.
///
/// This function is an implementation detail and SHOULD NOT be called directly!
#[doc(hidden)]
pub const fn decode<const LEN: usize>(strings: &[&[u8]]) -> [u8; LEN] {
    let mut i = 0;
    let mut buf = [0u8; LEN];
    let mut buf_pos = 0;
    while i < strings.len() {
        let mut pos = 0;
        while let Some((byte, new_pos)) = next_byte(strings[i], pos) {
            buf[buf_pos] = byte;
            buf_pos += 1;
            pos = new_pos;
        }
        i += 1;
    }
    if LEN != buf_pos {
        panic!("Length mismatch. Please report this bug.");
    }
    buf
}

/// Macro for converting sequence of string literals containing hex-encoded data
/// into an array of bytes.
#[macro_export]
macro_rules! hex {
    ($($s:literal)*) => {const {
        const STRINGS: &[&[u8]] = &[$( $crate::hex_literal::strip_hex_prefix($s.as_bytes()), )*];
        $crate::hex_literal::decode::<{ $crate::hex_literal::len(STRINGS) }>(STRINGS)
    }};
}
#[doc(hidden)] // Use `crate::hex` directly instead!
pub use crate::hex;

#[cfg(test)]
mod tests {
    #[test]
    fn single_literal() {
        assert_eq!(hex!("ff e4"), [0xff, 0xe4]);
    }

    #[test]
    fn empty() {
        let nothing: [u8; 0] = hex!();
        let empty_literals: [u8; 0] = hex!("" "" "");
        let expected: [u8; 0] = [];
        assert_eq!(nothing, expected);
        assert_eq!(empty_literals, expected);
    }

    #[test]
    fn upper_case() {
        assert_eq!(hex!("AE DF 04 B2"), [0xae, 0xdf, 0x04, 0xb2]);
        assert_eq!(hex!("FF BA 8C 00 01"), [0xff, 0xba, 0x8c, 0x00, 0x01]);
    }

    #[test]
    fn mixed_case() {
        assert_eq!(hex!("bF dd E4 Cd"), [0xbf, 0xdd, 0xe4, 0xcd]);
    }

    #[test]
    fn can_strip_prefix() {
        assert_eq!(hex!("0x1a2b3c"), [0x1a, 0x2b, 0x3c]);
        assert_eq!(hex!("0xa1" "0xb2" "0xc3"), [0xa1, 0xb2, 0xc3]);
    }

    #[test]
    fn multiple_literals() {
        assert_eq!(
            hex!(
                "01 dd f7 7f"
                "ee f0 d8"
            ),
            [0x01, 0xdd, 0xf7, 0x7f, 0xee, 0xf0, 0xd8]
        );
        assert_eq!(
            hex!(
                "ff"
                "e8 d0"
                ""
                "01 1f"
                "ab"
            ),
            [0xff, 0xe8, 0xd0, 0x01, 0x1f, 0xab]
        );
    }

    #[test]
    fn no_spacing() {
        assert_eq!(hex!("abf0d8bb0f14"), [0xab, 0xf0, 0xd8, 0xbb, 0x0f, 0x14]);
        assert_eq!(
            hex!("09FFd890cbcCd1d08F"),
            [0x09, 0xff, 0xd8, 0x90, 0xcb, 0xcc, 0xd1, 0xd0, 0x8f]
        );
    }

    #[test]
    fn allows_various_spacing() {
        // newlines
        assert_eq!(
            hex!(
                "f
                f
                d
                0
                e
                
                8
                "
            ),
            [0xff, 0xd0, 0xe8]
        );
        // tabs
        assert_eq!(hex!("9f	d		1		f07	3		01	"), [0x9f, 0xd1, 0xf0, 0x73, 0x01]);
        // spaces
        assert_eq!(hex!(" e    e d0  9 1   f  f  "), [0xee, 0xd0, 0x91, 0xff]);
    }

    #[test]
    const fn can_use_const() {
        const _: [u8; 4] = hex!("ff d3 01 7f");
    }
}
