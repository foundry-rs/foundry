//! Support for ASN.1 DER-encoded ECDSA signatures as specified in
//! [RFC5912 Appendix A].
//!
//! [RFC5912 Appendix A]: https://www.rfc-editor.org/rfc/rfc5912#appendix-A

use crate::{Error, Result};
use core::{
    fmt::{self, Debug},
    ops::{Add, Range},
};
use der::{asn1::UintRef, Decode, Encode, FixedTag, Length, Reader, Tag, Writer};
use elliptic_curve::{
    consts::U9,
    generic_array::{typenum::Unsigned, ArrayLength, GenericArray},
    FieldBytesSize, PrimeCurve,
};

#[cfg(feature = "alloc")]
use {
    alloc::{boxed::Box, vec::Vec},
    signature::SignatureEncoding,
    spki::{der::asn1::BitString, SignatureBitStringEncoding},
};

#[cfg(feature = "serde")]
use serdect::serde::{de, ser, Deserialize, Serialize};

/// Maximum overhead of an ASN.1 DER-encoded ECDSA signature for a given curve:
/// 9-bytes.
///
/// Includes 3-byte ASN.1 DER header:
///
/// - 1-byte: ASN.1 `SEQUENCE` tag (0x30)
/// - 2-byte: length
///
/// ...followed by two ASN.1 `INTEGER` values, which each have a header whose
/// maximum length is the following:
///
/// - 1-byte: ASN.1 `INTEGER` tag (0x02)
/// - 1-byte: length
/// - 1-byte: zero to indicate value is positive (`INTEGER` is signed)
pub type MaxOverhead = U9;

/// Maximum size of an ASN.1 DER encoded signature for the given elliptic curve.
pub type MaxSize<C> = <<FieldBytesSize<C> as Add>::Output as Add<MaxOverhead>>::Output;

/// Byte array containing a serialized ASN.1 signature
type SignatureBytes<C> = GenericArray<u8, MaxSize<C>>;

/// ASN.1 DER-encoded signature as specified in [RFC5912 Appendix A]:
///
/// ```text
/// ECDSA-Sig-Value ::= SEQUENCE {
///   r  INTEGER,
///   s  INTEGER
/// }
/// ```
///
/// [RFC5912 Appendix A]: https://www.rfc-editor.org/rfc/rfc5912#appendix-A
pub struct Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    /// ASN.1 DER-encoded signature data
    bytes: SignatureBytes<C>,

    /// Range of the `r` value within the signature
    r_range: Range<usize>,

    /// Range of the `s` value within the signature
    s_range: Range<usize>,
}

#[allow(clippy::len_without_is_empty)]
impl<C> Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    /// Parse signature from DER-encoded bytes.
    pub fn from_bytes(input: &[u8]) -> Result<Self> {
        let (r, s) = decode_der(input).map_err(|_| Error::new())?;

        if r.as_bytes().len() > C::FieldBytesSize::USIZE
            || s.as_bytes().len() > C::FieldBytesSize::USIZE
        {
            return Err(Error::new());
        }

        let r_range = find_scalar_range(input, r.as_bytes())?;
        let s_range = find_scalar_range(input, s.as_bytes())?;

        if s_range.end != input.len() {
            return Err(Error::new());
        }

        let mut bytes = SignatureBytes::<C>::default();
        bytes[..s_range.end].copy_from_slice(input);

        Ok(Signature {
            bytes,
            r_range,
            s_range,
        })
    }

    /// Create an ASN.1 DER encoded signature from big endian `r` and `s` scalar
    /// components.
    pub(crate) fn from_components(r: &[u8], s: &[u8]) -> der::Result<Self> {
        let r = UintRef::new(r)?;
        let s = UintRef::new(s)?;

        let mut bytes = SignatureBytes::<C>::default();
        let mut writer = der::SliceWriter::new(&mut bytes);

        writer.sequence((r.encoded_len()? + s.encoded_len()?)?, |seq| {
            seq.encode(&r)?;
            seq.encode(&s)
        })?;

        writer
            .finish()?
            .try_into()
            .map_err(|_| der::Tag::Sequence.value_error())
    }

    /// Borrow this signature as a byte slice
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes.as_slice()[..self.len()]
    }

    /// Serialize this signature as a boxed byte slice
    #[cfg(feature = "alloc")]
    pub fn to_bytes(&self) -> Box<[u8]> {
        self.as_bytes().to_vec().into_boxed_slice()
    }

    /// Get the length of the signature in bytes
    pub fn len(&self) -> usize {
        self.s_range.end
    }

    /// Get the `r` component of the signature (leading zeros removed)
    pub(crate) fn r(&self) -> &[u8] {
        &self.bytes[self.r_range.clone()]
    }

    /// Get the `s` component of the signature (leading zeros removed)
    pub(crate) fn s(&self) -> &[u8] {
        &self.bytes[self.s_range.clone()]
    }
}

impl<C> AsRef<[u8]> for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<C> Clone for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn clone(&self) -> Self {
        Self {
            bytes: self.bytes.clone(),
            r_range: self.r_range.clone(),
            s_range: self.s_range.clone(),
        }
    }
}

impl<C> Debug for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ecdsa::der::Signature<{:?}>(", C::default())?;

        for &byte in self.as_ref() {
            write!(f, "{:02X}", byte)?;
        }

        write!(f, ")")
    }
}

impl<'a, C> Decode<'a> for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn decode<R: Reader<'a>>(reader: &mut R) -> der::Result<Self> {
        let header = reader.peek_header()?;
        header.tag.assert_eq(Tag::Sequence)?;

        let mut buf = SignatureBytes::<C>::default();
        let len = (header.encoded_len()? + header.length)?;
        let slice = buf
            .get_mut(..usize::try_from(len)?)
            .ok_or_else(|| reader.error(Tag::Sequence.length_error().kind()))?;

        reader.read_into(slice)?;
        Self::from_bytes(slice).map_err(|_| Tag::Integer.value_error())
    }
}

impl<C> Encode for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn encoded_len(&self) -> der::Result<Length> {
        Length::try_from(self.len())
    }

    fn encode(&self, writer: &mut impl Writer) -> der::Result<()> {
        writer.write(self.as_bytes())
    }
}

impl<C> FixedTag for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    const TAG: Tag = Tag::Sequence;
}

impl<C> From<crate::Signature<C>> for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn from(sig: crate::Signature<C>) -> Signature<C> {
        sig.to_der()
    }
}

impl<C> TryFrom<&[u8]> for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    type Error = Error;

    fn try_from(input: &[u8]) -> Result<Self> {
        Self::from_bytes(input)
    }
}

impl<C> TryFrom<Signature<C>> for crate::Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    type Error = Error;

    fn try_from(sig: Signature<C>) -> Result<super::Signature<C>> {
        let mut bytes = super::SignatureBytes::<C>::default();
        let r_begin = C::FieldBytesSize::USIZE.saturating_sub(sig.r().len());
        let s_begin = bytes.len().saturating_sub(sig.s().len());
        bytes[r_begin..C::FieldBytesSize::USIZE].copy_from_slice(sig.r());
        bytes[s_begin..].copy_from_slice(sig.s());
        Self::try_from(bytes.as_slice())
    }
}

#[cfg(feature = "alloc")]
impl<C> From<Signature<C>> for Box<[u8]>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn from(signature: Signature<C>) -> Box<[u8]> {
        signature.to_vec().into_boxed_slice()
    }
}

#[cfg(feature = "alloc")]
impl<C> SignatureEncoding for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    type Repr = Box<[u8]>;

    fn to_vec(&self) -> Vec<u8> {
        self.as_bytes().into()
    }
}

#[cfg(feature = "alloc")]
impl<C> SignatureBitStringEncoding for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn to_bitstring(&self) -> der::Result<BitString> {
        BitString::new(0, self.to_vec())
    }
}

#[cfg(feature = "serde")]
impl<C> Serialize for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn serialize<S>(&self, serializer: S) -> core::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        serdect::slice::serialize_hex_upper_or_bin(&self.as_bytes(), serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de, C> Deserialize<'de> for Signature<C>
where
    C: PrimeCurve,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    fn deserialize<D>(deserializer: D) -> core::result::Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let mut buf = SignatureBytes::<C>::default();
        let slice = serdect::slice::deserialize_hex_or_bin(&mut buf, deserializer)?;
        Self::try_from(slice).map_err(de::Error::custom)
    }
}

/// Decode the `r` and `s` components of a DER-encoded ECDSA signature.
fn decode_der(der_bytes: &[u8]) -> der::Result<(UintRef<'_>, UintRef<'_>)> {
    let mut reader = der::SliceReader::new(der_bytes)?;
    let header = der::Header::decode(&mut reader)?;
    header.tag.assert_eq(der::Tag::Sequence)?;

    let ret = reader.read_nested(header.length, |reader| {
        let r = UintRef::decode(reader)?;
        let s = UintRef::decode(reader)?;
        Ok((r, s))
    })?;

    reader.finish(ret)
}

/// Locate the range within a slice at which a particular subslice is located
fn find_scalar_range(outer: &[u8], inner: &[u8]) -> Result<Range<usize>> {
    let outer_start = outer.as_ptr() as usize;
    let inner_start = inner.as_ptr() as usize;
    let start = inner_start
        .checked_sub(outer_start)
        .ok_or_else(Error::new)?;
    let end = start.checked_add(inner.len()).ok_or_else(Error::new)?;
    Ok(Range { start, end })
}

#[cfg(all(feature = "digest", feature = "hazmat"))]
impl<C> signature::PrehashSignature for Signature<C>
where
    C: PrimeCurve + crate::hazmat::DigestPrimitive,
    MaxSize<C>: ArrayLength<u8>,
    <FieldBytesSize<C> as Add>::Output: Add<MaxOverhead> + ArrayLength<u8>,
{
    type Digest = C::Digest;
}

#[cfg(all(test, feature = "arithmetic"))]
mod tests {
    use elliptic_curve::dev::MockCurve;

    type Signature = crate::Signature<MockCurve>;

    const EXAMPLE_SIGNATURE: [u8; 64] = [
        0xf3, 0xac, 0x80, 0x61, 0xb5, 0x14, 0x79, 0x5b, 0x88, 0x43, 0xe3, 0xd6, 0x62, 0x95, 0x27,
        0xed, 0x2a, 0xfd, 0x6b, 0x1f, 0x6a, 0x55, 0x5a, 0x7a, 0xca, 0xbb, 0x5e, 0x6f, 0x79, 0xc8,
        0xc2, 0xac, 0x8b, 0xf7, 0x78, 0x19, 0xca, 0x5, 0xa6, 0xb2, 0x78, 0x6c, 0x76, 0x26, 0x2b,
        0xf7, 0x37, 0x1c, 0xef, 0x97, 0xb2, 0x18, 0xe9, 0x6f, 0x17, 0x5a, 0x3c, 0xcd, 0xda, 0x2a,
        0xcc, 0x5, 0x89, 0x3,
    ];

    #[test]
    fn test_fixed_to_asn1_signature_roundtrip() {
        let signature1 = Signature::try_from(EXAMPLE_SIGNATURE.as_ref()).unwrap();

        // Convert to ASN.1 DER and back
        let asn1_signature = signature1.to_der();
        let signature2 = Signature::from_der(asn1_signature.as_ref()).unwrap();

        assert_eq!(signature1, signature2);
    }

    #[test]
    fn test_asn1_too_short_signature() {
        assert!(Signature::from_der(&[]).is_err());
        assert!(Signature::from_der(&[der::Tag::Sequence.into()]).is_err());
        assert!(Signature::from_der(&[der::Tag::Sequence.into(), 0x00]).is_err());
        assert!(Signature::from_der(&[
            der::Tag::Sequence.into(),
            0x03,
            der::Tag::Integer.into(),
            0x01,
            0x01
        ])
        .is_err());
    }

    #[test]
    fn test_asn1_non_der_signature() {
        // A minimal 8-byte ASN.1 signature parses OK.
        assert!(Signature::from_der(&[
            der::Tag::Sequence.into(),
            0x06, // length of below
            der::Tag::Integer.into(),
            0x01, // length of value
            0x01, // value=1
            der::Tag::Integer.into(),
            0x01, // length of value
            0x01, // value=1
        ])
        .is_ok());

        // But length fields that are not minimally encoded should be rejected, as they are not
        // valid DER, cf.
        // https://github.com/google/wycheproof/blob/2196000605e4/testvectors/ecdsa_secp256k1_sha256_test.json#L57-L66
        assert!(Signature::from_der(&[
            der::Tag::Sequence.into(),
            0x81, // extended length: 1 length byte to come
            0x06, // length of below
            der::Tag::Integer.into(),
            0x01, // length of value
            0x01, // value=1
            der::Tag::Integer.into(),
            0x01, // length of value
            0x01, // value=1
        ])
        .is_err());
    }
}
