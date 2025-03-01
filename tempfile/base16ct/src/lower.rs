use crate::{decode_inner, encoded_len, Error};
#[cfg(feature = "alloc")]
use crate::{decoded_len, String, Vec};

/// Decode a lower Base16 (hex) string into the provided destination buffer.
pub fn decode(src: impl AsRef<[u8]>, dst: &mut [u8]) -> Result<&[u8], Error> {
    decode_inner(src.as_ref(), dst, decode_nibble)
}

/// Decode a lower Base16 (hex) string into a byte vector.
#[cfg(feature = "alloc")]
pub fn decode_vec(input: impl AsRef<[u8]>) -> Result<Vec<u8>, Error> {
    let mut output = vec![0u8; decoded_len(input.as_ref())?];
    decode(input, &mut output)?;
    Ok(output)
}

/// Encode the input byte slice as lower Base16.
///
/// Writes the result into the provided destination slice, returning an
/// ASCII-encoded lower Base16 (hex) string value.
pub fn encode<'a>(src: &[u8], dst: &'a mut [u8]) -> Result<&'a [u8], Error> {
    let dst = dst
        .get_mut(..encoded_len(src))
        .ok_or(Error::InvalidLength)?;
    for (src, dst) in src.iter().zip(dst.chunks_exact_mut(2)) {
        dst[0] = encode_nibble(src >> 4);
        dst[1] = encode_nibble(src & 0x0f);
    }
    Ok(dst)
}

/// Encode input byte slice into a [`&str`] containing lower Base16 (hex).
pub fn encode_str<'a>(src: &[u8], dst: &'a mut [u8]) -> Result<&'a str, Error> {
    encode(src, dst).map(|r| unsafe { core::str::from_utf8_unchecked(r) })
}

/// Encode input byte slice into a [`String`] containing lower Base16 (hex).
///
/// # Panics
/// If `input` length is greater than `usize::MAX/2`.
#[cfg(feature = "alloc")]
pub fn encode_string(input: &[u8]) -> String {
    let elen = encoded_len(input);
    let mut dst = vec![0u8; elen];
    let res = encode(input, &mut dst).expect("dst length is correct");

    debug_assert_eq!(elen, res.len());
    unsafe { String::from_utf8_unchecked(dst) }
}

/// Decode a single nibble of lower hex
#[inline(always)]
fn decode_nibble(src: u8) -> u16 {
    // 0-9  0x30-0x39
    // A-F  0x41-0x46 or a-f  0x61-0x66
    let byte = src as i16;
    let mut ret: i16 = -1;

    // 0-9  0x30-0x39
    // if (byte > 0x2f && byte < 0x3a) ret += byte - 0x30 + 1; // -47
    ret += (((0x2fi16 - byte) & (byte - 0x3a)) >> 8) & (byte - 47);
    // a-f  0x61-0x66
    // if (byte > 0x60 && byte < 0x67) ret += byte - 0x61 + 10 + 1; // -86
    ret += (((0x60i16 - byte) & (byte - 0x67)) >> 8) & (byte - 86);

    ret as u16
}

/// Encode a single nibble of hex
#[inline(always)]
fn encode_nibble(src: u8) -> u8 {
    let mut ret = src as i16 + 0x30;
    // 0-9  0x30-0x39
    // a-f  0x61-0x66
    ret += ((0x39i16 - ret) >> 8) & (0x61i16 - 0x3a);
    ret as u8
}
