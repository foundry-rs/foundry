use crate::{
    FormatError, MAX_CA_BUNDLE, MAX_DOCUMENT_SIZE, MAX_PAYLOAD_SIZE, MAX_PCRS,
    P384_FIXED_SIGNATURE_SIZE, ParsedAttestation, Pcr,
};
use alloc::{collections::BTreeMap, vec::Vec};
use arrayvec::ArrayVec;
use core::num::NonZeroU64;
use minicbor::{
    Decoder, Encoder,
    data::{Tag, Type},
};
use serde::Deserialize;

type Result<T> = core::result::Result<T, crate::Error>;

/// The COSE_Sign1 array: protected headers, unprotected headers, payload, signature.
#[derive(Debug, Deserialize)]
struct CoseSign1<'a>(
    #[serde(borrow)] &'a [u8],
    EmptyHeaders,
    #[serde(borrow)] &'a [u8],
    #[serde(borrow)] &'a [u8],
);

/// Reject any unprotected entry before decoding its value.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyHeaders {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttestationDocument<'a> {
    module_id: &'a str,
    digest: &'a str,
    timestamp: NonZeroU64,
    #[serde(borrow, with = "serde_with::rust::maps_duplicate_key_is_error")]
    pcrs: BTreeMap<u8, &'a [u8]>,
    #[serde(borrow)]
    certificate: &'a [u8],
    #[serde(borrow)]
    cabundle: ArrayVec<&'a [u8], MAX_CA_BUNDLE>,
    #[serde(borrow, default)]
    public_key: Option<&'a [u8]>,
    #[serde(borrow, default)]
    user_data: Option<&'a [u8]>,
    #[serde(borrow, default)]
    nonce: Option<&'a [u8]>,
}

/// Decode into borrowed fields before allocating the public, owned representation.
///
/// This profile rejects unknown payload fields, payload tags, and chunked byte/text strings.
/// The envelope and protected header must be definite-length; the empty unprotected map and
/// payload collections may be indefinite-length. These are acceptance-rule changes from the
/// previous Ciborium parser.
/// Collection work is bounded by the fixed-capacity CA bundle and duplicate-free `u8` PCR
/// keys (at most 256 entries during decoding; validation requires indices below 32). Only the
/// bounded PCR map allocates while decoding; no generic CBOR value tree is built.
pub(crate) fn parse_attestation(document: &[u8]) -> Result<ParsedAttestation> {
    if document.len() > MAX_DOCUMENT_SIZE {
        return Err(FormatError::DocumentTooLarge.into());
    }
    let mut decoder = Decoder::new(document);
    if decoder.datatype().map_err(cose_error)? == Type::Tag
        && decoder.tag().map_err(cose_error)? != Tag::new(18)
    {
        return Err(FormatError::InvalidCoseTag.into());
    }
    decode_exact::<CoseSign1<'_>>(decoder, FormatError::InvalidCoseStructure)?.try_into()
}

impl TryFrom<CoseSign1<'_>> for ParsedAttestation {
    type Error = crate::Error;

    fn try_from(CoseSign1(protected, _, payload, signature): CoseSign1<'_>) -> Result<Self> {
        validate_protected(protected).map_err(|_| FormatError::InvalidProtectedHeader)?;
        if payload.is_empty() || payload.len() > MAX_PAYLOAD_SIZE {
            return Err(FormatError::InvalidField("payload").into());
        }
        let signature: [u8; P384_FIXED_SIGNATURE_SIZE] = signature
            .try_into()
            .map_err(|_| FormatError::InvalidField("signature"))?;
        let AttestationDocument {
            module_id,
            digest,
            timestamp,
            pcrs,
            certificate,
            cabundle,
            public_key,
            user_data,
            nonce,
        } = decode_exact(Decoder::new(payload), FormatError::InvalidPayload)?;
        if module_id.is_empty()
            || digest != "SHA384"
            || pcrs.is_empty()
            || pcrs.keys().any(|index| usize::from(*index) >= MAX_PCRS)
            || pcrs
                .values()
                .any(|value| !matches!(value.len(), 32 | 48 | 64))
            || !(1..=1024).contains(&certificate.len())
            || cabundle.is_empty()
            || cabundle
                .iter()
                .any(|value| !(1..=1024).contains(&value.len()))
            || public_key.is_some_and(|value| !(1..=1024).contains(&value.len()))
            || user_data.is_some_and(|value| value.len() > 512)
            || nonce.is_some_and(|value| value.len() > 512)
        {
            return Err(FormatError::InvalidPayload.into());
        }
        Ok(Self {
            module_id: module_id.into(),
            timestamp: timestamp.get(),
            pcrs: pcrs
                .into_iter()
                .map(|(index, value)| Pcr {
                    index,
                    value: value.to_vec(),
                })
                .collect(),
            certificate: certificate.to_vec(),
            cabundle: cabundle.into_iter().map(|value| value.to_vec()).collect(),
            public_key: public_key.map_or_else(Vec::new, |value| value.to_vec()),
            user_data: user_data.map_or_else(Vec::new, |value| value.to_vec()),
            nonce: nonce.map_or_else(Vec::new, |value| value.to_vec()),
            protected: protected.to_vec(),
            payload: payload.to_vec(),
            signature,
        })
    }
}

/// Encodes the COSE Sig_structure used for signature verification:
/// `["Signature1", protected, external_aad, payload]`, with empty external AAD.
/// The protected header and payload are embedded as their original byte strings.
pub(crate) fn encode_sig_structure(protected: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .array(4)
        .and_then(|e| e.str("Signature1"))
        .and_then(|e| e.bytes(protected))
        .and_then(|e| e.bytes(&[]))
        .and_then(|e| e.bytes(payload))
        .map_err(|_| FormatError::InvalidCbor)?;
    Ok(encoder.into_writer())
}

/// Decode one borrowed value and require that it consumes the remaining input.
fn decode_exact<'de, T: Deserialize<'de>>(decoder: Decoder<'de>, error: FormatError) -> Result<T> {
    let mut deserializer = minicbor_serde::Deserializer::from(decoder);
    let value = T::deserialize(&mut deserializer).map_err(|_| error)?;
    if deserializer.decoder().position() != deserializer.decoder().input().len() {
        return Err(error.into());
    }
    Ok(value)
}

fn cose_error(_: minicbor::decode::Error) -> crate::Error {
    FormatError::InvalidCoseStructure.into()
}

/// The header has a fixed schema: reject extra entries before traversing their contents.
fn validate_protected(input: &[u8]) -> core::result::Result<(), minicbor::decode::Error> {
    let mut decoder = Decoder::new(input);
    if decoder.map()? != Some(1)
        || decoder.u8()? != 1
        || decoder.i8()? != -35
        || decoder.position() != input.len()
    {
        return Err(minicbor::decode::Error::message("expected {1: -35}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use minicbor::{Decoder, Encoder, data::Tag};

    fn payload(indefinite: bool, duplicate_module: bool, include_null_optionals: bool) -> Vec<u8> {
        let mut e = Encoder::new(Vec::new());
        if indefinite {
            e.begin_map().unwrap();
        } else {
            e.map(6 + u64::from(duplicate_module) + 3 * u64::from(include_null_optionals))
                .unwrap();
        }
        e.str("module_id").unwrap().str("module").unwrap();
        if duplicate_module {
            e.str("module_id").unwrap().str("again").unwrap();
        }
        e.str("digest").unwrap().str("SHA384").unwrap();
        e.str("timestamp").unwrap().u64(1).unwrap();
        e.str("pcrs").unwrap();
        if indefinite {
            e.begin_map().unwrap();
        } else {
            e.map(2).unwrap();
        }
        e.u8(7).unwrap().bytes(&[7; 48]).unwrap();
        e.u8(0).unwrap().bytes(&[0; 48]).unwrap();
        if indefinite {
            e.end().unwrap();
        }
        e.str("certificate").unwrap().bytes(&[0x30]).unwrap();
        e.str("cabundle").unwrap();
        if indefinite {
            e.begin_array()
                .unwrap()
                .bytes(&[0x30])
                .unwrap()
                .end()
                .unwrap();
        } else {
            e.array(1).unwrap().bytes(&[0x30]).unwrap();
        }
        if include_null_optionals {
            e.str("public_key").unwrap().null().unwrap();
            e.str("user_data").unwrap().null().unwrap();
            e.str("nonce").unwrap().null().unwrap();
        }
        if indefinite {
            e.end().unwrap();
        }
        e.into_writer()
    }

    fn document(tagged: bool, indefinite: bool, duplicate_module: bool) -> Vec<u8> {
        wrap_payload(
            tagged,
            indefinite,
            &payload(indefinite, duplicate_module, false),
        )
    }

    fn wrap_payload(tagged: bool, indefinite: bool, payload: &[u8]) -> Vec<u8> {
        let protected = {
            let mut e = Encoder::new(Vec::new());
            e.map(1).unwrap().u8(1).unwrap().i8(-35).unwrap();
            e.into_writer()
        };
        let mut e = Encoder::new(Vec::new());
        if tagged {
            e.tag(Tag::new(18)).unwrap();
        }
        if indefinite {
            e.begin_array().unwrap();
        } else {
            e.array(4).unwrap();
        }
        e.bytes(&protected)
            .unwrap()
            .map(0)
            .unwrap()
            .bytes(payload)
            .unwrap()
            .bytes(&[0; P384_FIXED_SIGNATURE_SIZE])
            .unwrap();
        if indefinite {
            e.end().unwrap();
        }
        e.into_writer()
    }

    #[derive(Clone)]
    struct TestField {
        name: &'static str,
        value: Vec<u8>,
    }

    fn encoded_value(encode: impl FnOnce(&mut Encoder<Vec<u8>>)) -> Vec<u8> {
        let mut encoder = Encoder::new(Vec::new());
        encode(&mut encoder);
        encoder.into_writer()
    }

    fn text_value(value: &str) -> Vec<u8> {
        encoded_value(|encoder| {
            encoder.str(value).unwrap();
        })
    }

    fn bytes_value(value: &[u8]) -> Vec<u8> {
        encoded_value(|encoder| {
            encoder.bytes(value).unwrap();
        })
    }

    fn unsigned_value(value: u64) -> Vec<u8> {
        encoded_value(|encoder| {
            encoder.u64(value).unwrap();
        })
    }

    fn null_value() -> Vec<u8> {
        encoded_value(|encoder| {
            encoder.null().unwrap();
        })
    }

    fn bool_value(value: bool) -> Vec<u8> {
        encoded_value(|encoder| {
            encoder.bool(value).unwrap();
        })
    }

    fn pcrs_value(entries: &[(u8, Vec<u8>)]) -> Vec<u8> {
        encoded_value(|encoder| {
            encoder.map(entries.len() as u64).unwrap();
            for (index, value) in entries {
                encoder.u8(*index).unwrap().bytes(value).unwrap();
            }
        })
    }

    fn cabundle_value(certificates: &[Vec<u8>]) -> Vec<u8> {
        encoded_value(|encoder| {
            encoder.array(certificates.len() as u64).unwrap();
            for certificate in certificates {
                encoder.bytes(certificate).unwrap();
            }
        })
    }

    fn default_fields() -> Vec<TestField> {
        vec![
            TestField {
                name: "module_id",
                value: text_value("module"),
            },
            TestField {
                name: "digest",
                value: text_value("SHA384"),
            },
            TestField {
                name: "timestamp",
                value: unsigned_value(1),
            },
            TestField {
                name: "pcrs",
                value: pcrs_value(&[(0, vec![0; 48])]),
            },
            TestField {
                name: "certificate",
                value: bytes_value(&[0x30]),
            },
            TestField {
                name: "cabundle",
                value: cabundle_value(&[vec![0x30]]),
            },
        ]
    }

    fn set_field(fields: &mut [TestField], name: &str, value: Vec<u8>) {
        fields
            .iter_mut()
            .find(|field| field.name == name)
            .unwrap_or_else(|| panic!("missing test field {name}"))
            .value = value;
    }

    fn encode_payload(fields: &[TestField]) -> Vec<u8> {
        let mut encoder = Encoder::new(Vec::new());
        encoder.map(fields.len() as u64).unwrap();
        let mut output = encoder.into_writer();
        for field in fields {
            let mut key = Encoder::new(Vec::new());
            key.str(field.name).unwrap();
            output.extend_from_slice(&key.into_writer());
            output.extend_from_slice(&field.value);
        }
        output
    }

    fn parse_fields(fields: &[TestField]) -> Result<ParsedAttestation> {
        parse_attestation(&wrap_payload(false, false, &encode_payload(fields)))
    }

    fn assert_field_error(fields: &[TestField], expected: FormatError) {
        assert_eq!(
            parse_fields(fields).unwrap_err(),
            crate::Error::InvalidFormat(expected)
        );
    }

    fn payload_with_exact_len(target: usize) -> Vec<u8> {
        let mut fields = default_fields();
        let base_len = encode_payload(&fields).len();
        let mut module_len =
            target.checked_sub(base_len).expect("target fits payload") + "module".len();

        for _ in 0..4 {
            set_field(
                &mut fields,
                "module_id",
                text_value(&"x".repeat(module_len)),
            );
            let payload = encode_payload(&fields);
            match payload.len().cmp(&target) {
                core::cmp::Ordering::Equal => return payload,
                core::cmp::Ordering::Less => module_len += target - payload.len(),
                core::cmp::Ordering::Greater => module_len -= payload.len() - target,
            }
        }
        panic!("failed to construct an exact-length payload")
    }

    fn wrap_payload_with_unprotected_padding(payload: &[u8], padding_len: usize) -> Vec<u8> {
        let protected = {
            let mut encoder = Encoder::new(Vec::new());
            encoder.map(1).unwrap().u8(1).unwrap().i8(-35).unwrap();
            encoder.into_writer()
        };
        let mut encoder = Encoder::new(Vec::new());
        encoder
            .array(4)
            .unwrap()
            .bytes(&protected)
            .unwrap()
            .map(1)
            .unwrap()
            .u8(0)
            .unwrap()
            .bytes(&vec![0; padding_len])
            .unwrap()
            .bytes(payload)
            .unwrap()
            .bytes(&[0; P384_FIXED_SIGNATURE_SIZE])
            .unwrap();
        encoder.into_writer()
    }

    fn document_with_exact_len(target: usize) -> Vec<u8> {
        let payload = encode_payload(&default_fields());
        let base_len = wrap_payload_with_unprotected_padding(&payload, 0).len();
        let mut padding_len = target.checked_sub(base_len).expect("target fits document");

        for _ in 0..4 {
            let document = wrap_payload_with_unprotected_padding(&payload, padding_len);
            match document.len().cmp(&target) {
                core::cmp::Ordering::Equal => return document,
                core::cmp::Ordering::Less => padding_len += target - document.len(),
                core::cmp::Ordering::Greater => padding_len -= document.len() - target,
            }
        }
        panic!("failed to construct an exact-length document")
    }

    #[test]
    fn parses_tagged_and_untagged_documents() {
        for tagged in [false, true] {
            let parsed = parse_attestation(&document(tagged, false, false)).unwrap();
            assert_eq!(parsed.module_id, "module");
            assert_eq!(parsed.timestamp, 1);
            assert_eq!(parsed.pcrs[0].index, 0);
            assert_eq!(parsed.pcrs[1].index, 7);
            assert_eq!(parsed.signature_count(), 2);
        }
    }

    #[test]
    fn accepts_indefinite_payload_collections_but_rejects_indefinite_envelope() {
        let parsed =
            parse_attestation(&wrap_payload(true, false, &payload(true, false, false))).unwrap();
        assert_eq!(parsed.pcrs.len(), 2);
        assert_eq!(parsed.cabundle, vec![vec![0x30]]);
        assert!(parse_attestation(&document(true, true, false)).is_err());
    }

    #[test]
    fn accepts_null_optional_fields_as_empty() {
        let payload = payload(false, false, true);
        let parsed = parse_attestation(&wrap_payload(false, false, &payload)).unwrap();
        assert!(parsed.public_key.is_empty());
        assert!(parsed.user_data.is_empty());
        assert!(parsed.nonce.is_empty());
    }

    #[test]
    fn rejects_duplicate_null_optional_fields() {
        for name in ["public_key", "user_data", "nonce"] {
            let mut fields = default_fields();
            for _ in 0..2 {
                fields.push(TestField {
                    name,
                    value: null_value(),
                });
            }
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
    }

    #[test]
    fn rejects_tagged_fields_and_arrays_in_byte_fields() {
        for field in default_fields() {
            let mut fields = default_fields();
            let mut tagged = encoded_value(|e| {
                e.tag(Tag::new(100)).unwrap();
            });
            tagged.extend_from_slice(&field.value);
            set_field(&mut fields, field.name, tagged);
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
        for name in ["public_key", "user_data", "nonce"] {
            for value in [
                vec![0x81, 0x01],         // an integer array is not a byte string
                vec![0xd8, 100, 0x41, 1], // a tag is not a byte string
            ] {
                let mut fields = default_fields();
                fields.push(TestField { name, value });
                assert_field_error(&fields, FormatError::InvalidPayload);
            }
        }
    }

    #[test]
    fn rejects_tagged_collection_entries() {
        for tag_key in [false, true] {
            let mut fields = default_fields();
            set_field(
                &mut fields,
                "pcrs",
                encoded_value(|e| {
                    e.map(1).unwrap();
                    if tag_key {
                        e.tag(Tag::new(100)).unwrap();
                    }
                    e.u8(0).unwrap();
                    if !tag_key {
                        e.tag(Tag::new(100)).unwrap();
                    }
                    e.bytes(&[0; 48]).unwrap();
                }),
            );
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
        let mut fields = default_fields();
        set_field(
            &mut fields,
            "cabundle",
            encoded_value(|e| {
                e.array(1)
                    .unwrap()
                    .tag(Tag::new(100))
                    .unwrap()
                    .bytes(&[1])
                    .unwrap();
            }),
        );
        assert_field_error(&fields, FormatError::InvalidPayload);
    }

    #[test]
    fn non_text_keys_cannot_supply_required_fields() {
        let mut fields = default_fields();
        fields.retain(|field| field.name != "module_id");
        for key in [unsigned_value(0), bytes_value(b"module_id")] {
            let mut payload = encode_payload(&fields);
            payload[0] = 0xa6;
            payload.extend(key);
            payload.extend(text_value("module"));
            assert_eq!(
                parse_attestation(&wrap_payload(false, false, &payload)).unwrap_err(),
                crate::Error::InvalidFormat(FormatError::InvalidPayload)
            );
        }
    }

    #[test]
    fn rejects_chunked_byte_and_text_fields() {
        for (name, value) in [
            (
                "module_id",
                encoded_value(|e| {
                    e.begin_str()
                        .unwrap()
                        .str("mod")
                        .unwrap()
                        .str("ule")
                        .unwrap()
                        .end()
                        .unwrap();
                }),
            ),
            (
                "certificate",
                encoded_value(|e| {
                    e.begin_bytes()
                        .unwrap()
                        .bytes(&[0x30])
                        .unwrap()
                        .end()
                        .unwrap();
                }),
            ),
        ] {
            let mut fields = default_fields();
            set_field(&mut fields, name, value);
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
        let payload = encode_payload(&default_fields());
        let parsed = parse_attestation(&wrap_payload(true, false, &payload)).unwrap();
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn rejects_unknown_fields_before_reading_the_value() {
        let mut payload = encode_payload(&default_fields());
        payload[0] += 1;
        payload.extend(text_value("future_field")); // Deliberately omit its value.
        let mut decoder = minicbor_serde::Deserializer::new(&payload);
        let error = AttestationDocument::deserialize(&mut decoder).unwrap_err();
        assert!(alloc::format!("{error}").contains("unknown field"));
    }

    #[test]
    fn rejects_undefined_optionals_and_tagged_integers() {
        for name in ["public_key", "user_data", "nonce"] {
            let mut fields = default_fields();
            fields.push(TestField {
                name,
                value: vec![0xf7],
            });
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
        let mut fields = default_fields();
        set_field(&mut fields, "timestamp", vec![0xc2, 0x41, 1]);
        assert_field_error(&fields, FormatError::InvalidPayload);
    }

    #[test]
    fn rejects_missing_mandatory_fields() {
        for name in [
            "module_id",
            "digest",
            "timestamp",
            "pcrs",
            "certificate",
            "cabundle",
        ] {
            let mut fields = default_fields();
            fields.retain(|field| field.name != name);
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
    }

    #[test]
    fn rejects_duplicate_recognized_fields_matrix() {
        for name in [
            "module_id",
            "digest",
            "timestamp",
            "pcrs",
            "certificate",
            "cabundle",
        ] {
            let mut fields = default_fields();
            let duplicate = fields
                .iter()
                .find(|field| field.name == name)
                .unwrap()
                .clone();
            fields.push(duplicate);
            assert_field_error(&fields, FormatError::InvalidPayload);
        }

        for name in ["public_key", "user_data", "nonce"] {
            let mut fields = default_fields();
            fields.push(TestField {
                name,
                value: bytes_value(&[1]),
            });
            fields.push(TestField {
                name,
                value: null_value(),
            });
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
    }

    #[test]
    fn rejects_wrong_types_for_all_fields() {
        for name in [
            "module_id",
            "digest",
            "timestamp",
            "pcrs",
            "certificate",
            "cabundle",
        ] {
            let mut fields = default_fields();
            set_field(&mut fields, name, null_value());
            assert_field_error(&fields, FormatError::InvalidPayload);
        }

        for name in ["public_key", "user_data", "nonce"] {
            let mut fields = default_fields();
            fields.push(TestField {
                name,
                value: bool_value(false),
            });
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
    }

    #[test]
    fn enforces_scalar_and_byte_string_bounds() {
        let mut fields = default_fields();
        set_field(&mut fields, "module_id", text_value(""));
        assert_field_error(&fields, FormatError::InvalidPayload);

        let mut fields = default_fields();
        set_field(&mut fields, "digest", text_value("SHA256"));
        assert_field_error(&fields, FormatError::InvalidPayload);

        let mut fields = default_fields();
        set_field(&mut fields, "timestamp", unsigned_value(0));
        assert_field_error(&fields, FormatError::InvalidPayload);

        for (name, min, max) in [
            ("certificate", 1, 1_024),
            ("public_key", 1, 1_024),
            ("user_data", 0, 512),
            ("nonce", 0, 512),
        ] {
            if min > 0 {
                let mut fields = default_fields();
                if matches!(name, "public_key" | "user_data" | "nonce") {
                    fields.push(TestField {
                        name,
                        value: bytes_value(&[]),
                    });
                } else {
                    set_field(&mut fields, name, bytes_value(&[]));
                }
                assert_field_error(&fields, FormatError::InvalidPayload);
            }

            let mut fields = default_fields();
            let oversized = bytes_value(&vec![0; max + 1]);
            if matches!(name, "public_key" | "user_data" | "nonce") {
                fields.push(TestField {
                    name,
                    value: oversized,
                });
            } else {
                set_field(&mut fields, name, oversized);
            }
            assert_field_error(&fields, FormatError::InvalidPayload);
        }

        let mut fields = default_fields();
        fields.push(TestField {
            name: "public_key",
            value: bytes_value(&[1]),
        });
        fields.push(TestField {
            name: "user_data",
            value: bytes_value(&[]),
        });
        fields.push(TestField {
            name: "nonce",
            value: bytes_value(&[]),
        });
        let parsed = parse_fields(&fields).unwrap();
        assert_eq!(parsed.public_key, vec![1]);
        assert!(parsed.user_data.is_empty());
        assert!(parsed.nonce.is_empty());
    }

    #[test]
    fn accepts_exact_field_maxima_and_policy_neutral_values() {
        let module_id = "\0arbitrary module 🦀";
        let pcr_value = vec![0xff; 64];
        let certificate = vec![0xa5; 1_024];
        let ca_certificate = vec![0x5a; 1_024];
        let public_key = vec![0x11; 1_024];
        let user_data = vec![0x22; 512];
        let nonce = vec![0x33; 512];
        let mut fields = default_fields();
        set_field(&mut fields, "module_id", text_value(module_id));
        set_field(&mut fields, "timestamp", unsigned_value(u64::MAX));
        set_field(&mut fields, "pcrs", pcrs_value(&[(31, pcr_value.clone())]));
        set_field(&mut fields, "certificate", bytes_value(&certificate));
        set_field(
            &mut fields,
            "cabundle",
            cabundle_value(core::slice::from_ref(&ca_certificate)),
        );
        fields.push(TestField {
            name: "public_key",
            value: bytes_value(&public_key),
        });
        fields.push(TestField {
            name: "user_data",
            value: bytes_value(&user_data),
        });
        fields.push(TestField {
            name: "nonce",
            value: bytes_value(&nonce),
        });

        let parsed = parse_fields(&fields).unwrap();
        assert_eq!(parsed.module_id, module_id);
        assert_eq!(parsed.timestamp, u64::MAX);
        assert_eq!(
            parsed.pcrs,
            vec![Pcr {
                index: 31,
                value: pcr_value
            }]
        );
        assert_eq!(parsed.certificate, certificate);
        assert_eq!(parsed.cabundle, vec![ca_certificate]);
        assert_eq!(parsed.public_key, public_key);
        assert_eq!(parsed.user_data, user_data);
        assert_eq!(parsed.nonce, nonce);
    }

    #[test]
    fn enforces_pcr_constraints_and_accepts_all_value_lengths() {
        let mut fields = default_fields();
        set_field(
            &mut fields,
            "pcrs",
            pcrs_value(&[(2, vec![2; 64]), (0, vec![0; 32]), (1, vec![1; 48])]),
        );
        let parsed = parse_fields(&fields).unwrap();
        assert_eq!(
            parsed
                .pcrs
                .iter()
                .map(|pcr| (pcr.index, pcr.value.len()))
                .collect::<Vec<_>>(),
            vec![(0, 32), (1, 48), (2, 64)]
        );

        for invalid_len in [0, 31, 33, 47, 49, 63, 65] {
            let mut fields = default_fields();
            set_field(
                &mut fields,
                "pcrs",
                pcrs_value(&[(0, vec![0; invalid_len])]),
            );
            assert_field_error(&fields, FormatError::InvalidPayload);
        }

        let mut fields = default_fields();
        set_field(&mut fields, "pcrs", pcrs_value(&[]));
        assert_field_error(&fields, FormatError::InvalidPayload);

        let mut fields = default_fields();
        set_field(&mut fields, "pcrs", pcrs_value(&[(32, vec![0; 48])]));
        assert_field_error(&fields, FormatError::InvalidPayload);

        let mut fields = default_fields();
        set_field(
            &mut fields,
            "pcrs",
            pcrs_value(&[(7, vec![0; 48]), (7, vec![1; 48])]),
        );
        assert_field_error(&fields, FormatError::InvalidPayload);

        for malformed_pcrs in [
            encoded_value(|encoder| {
                encoder
                    .map(1)
                    .unwrap()
                    .str("zero")
                    .unwrap()
                    .bytes(&[0; 48])
                    .unwrap();
            }),
            encoded_value(|encoder| {
                encoder
                    .map(1)
                    .unwrap()
                    .u8(0)
                    .unwrap()
                    .str("not bytes")
                    .unwrap();
            }),
        ] {
            let mut fields = default_fields();
            set_field(&mut fields, "pcrs", malformed_pcrs);
            assert_field_error(&fields, FormatError::InvalidPayload);
        }
    }

    #[test]
    fn enforces_pcr_and_cabundle_count_limits() {
        let pcrs = (0..MAX_PCRS)
            .map(|index| (index as u8, vec![index as u8; 48]))
            .collect::<Vec<_>>();
        let mut fields = default_fields();
        set_field(&mut fields, "pcrs", pcrs_value(&pcrs));
        assert_eq!(parse_fields(&fields).unwrap().pcrs.len(), MAX_PCRS);

        let pcrs = (0..=MAX_PCRS)
            .map(|index| (index as u8, vec![index as u8; 48]))
            .collect::<Vec<_>>();
        let mut fields = default_fields();
        set_field(&mut fields, "pcrs", pcrs_value(&pcrs));
        assert_field_error(&fields, FormatError::InvalidPayload);

        let certificates = (0..MAX_CA_BUNDLE)
            .map(|index| vec![index as u8])
            .collect::<Vec<_>>();
        let mut fields = default_fields();
        set_field(&mut fields, "cabundle", cabundle_value(&certificates));
        assert_eq!(parse_fields(&fields).unwrap().cabundle.len(), MAX_CA_BUNDLE);

        let certificates = (0..=MAX_CA_BUNDLE)
            .map(|index| vec![index as u8])
            .collect::<Vec<_>>();
        let mut fields = default_fields();
        set_field(&mut fields, "cabundle", cabundle_value(&certificates));
        assert_field_error(&fields, FormatError::InvalidPayload);
    }

    #[test]
    fn enforces_cabundle_entry_bounds() {
        let mut fields = default_fields();
        set_field(&mut fields, "cabundle", cabundle_value(&[]));
        assert_field_error(&fields, FormatError::InvalidPayload);

        for invalid in [Vec::new(), vec![0; 1_025]] {
            let mut fields = default_fields();
            set_field(
                &mut fields,
                "cabundle",
                cabundle_value(core::slice::from_ref(&invalid)),
            );
            assert_field_error(&fields, FormatError::InvalidPayload);
        }

        let mut fields = default_fields();
        set_field(
            &mut fields,
            "cabundle",
            encoded_value(|encoder| {
                encoder.array(1).unwrap().bool(false).unwrap();
            }),
        );
        assert_field_error(&fields, FormatError::InvalidPayload);
    }

    #[test]
    fn rejects_unknown_payload_fields() {
        let mut fields = default_fields();
        fields.push(TestField {
            name: "future_field",
            value: encoded_value(|encoder| {
                encoder
                    .array(2)
                    .unwrap()
                    .map(1)
                    .unwrap()
                    .str("nested")
                    .unwrap()
                    .u64(42)
                    .unwrap()
                    .tag(Tag::new(1_000))
                    .unwrap()
                    .null()
                    .unwrap();
            }),
        });
        assert_field_error(&fields, FormatError::InvalidPayload);
    }

    #[test]
    fn rejects_unknown_non_text_payload_keys() {
        let mut payload = encode_payload(&default_fields());
        assert_eq!(payload[0], 0xa6, "fixture must start with a six-entry map");
        payload[0] = 0xa7;
        let mut unknown = Encoder::new(Vec::new());
        unknown
            .u8(42)
            .unwrap()
            .array(2)
            .unwrap()
            .bool(true)
            .unwrap()
            .null()
            .unwrap();
        payload.extend_from_slice(&unknown.into_writer());

        assert!(parse_attestation(&wrap_payload(false, false, &payload)).is_err());
    }

    #[test]
    fn enforces_payload_and_document_size_limits() {
        let payload = payload_with_exact_len(MAX_PAYLOAD_SIZE);
        assert_eq!(payload.len(), MAX_PAYLOAD_SIZE);
        assert!(parse_attestation(&wrap_payload(false, false, &payload)).is_ok());

        let oversized_payload = vec![0; MAX_PAYLOAD_SIZE + 1];
        assert_eq!(
            parse_attestation(&wrap_payload(false, false, &oversized_payload)).unwrap_err(),
            crate::Error::InvalidFormat(FormatError::InvalidField("payload"))
        );

        let document = document_with_exact_len(MAX_DOCUMENT_SIZE);
        assert_eq!(document.len(), MAX_DOCUMENT_SIZE);
        assert_eq!(
            parse_attestation(&document).unwrap_err(),
            crate::Error::InvalidFormat(FormatError::InvalidCoseStructure)
        );

        let mut oversized_document = document;
        oversized_document.push(0);
        assert_eq!(
            parse_attestation(&oversized_document).unwrap_err(),
            crate::Error::InvalidFormat(FormatError::DocumentTooLarge)
        );
    }

    #[test]
    fn rejects_duplicate_recognized_fields() {
        let error = parse_attestation(&document(false, false, true)).unwrap_err();
        assert_eq!(
            error,
            crate::Error::InvalidFormat(FormatError::InvalidPayload)
        );
    }

    #[test]
    fn rejects_trailing_bytes_and_wrong_tag() {
        let mut trailing = document(false, false, false);
        trailing.push(0);
        assert!(parse_attestation(&trailing).is_err());

        let mut wrong_tag = document(true, false, false);
        wrong_tag[0] = 0xd1; // tag 17 instead of tag 18
        assert_eq!(
            parse_attestation(&wrong_tag).unwrap_err(),
            crate::Error::InvalidFormat(FormatError::InvalidCoseTag)
        );
    }

    #[test]
    fn sig_structure_uses_exact_protected_and_payload_bytes() {
        let encoded = encode_sig_structure(&[0xa1, 1, 0x38, 0x22], &[1, 2, 3]).unwrap();
        assert_eq!(
            encoded,
            b"\x84\x6aSignature1\x44\xa1\x01\x38\x22\x40\x43\x01\x02\x03"
        );
        let mut d = Decoder::new(&encoded);
        assert_eq!(d.array().unwrap(), Some(4));
        assert_eq!(d.str().unwrap(), "Signature1");
        assert_eq!(d.bytes().unwrap(), &[0xa1, 1, 0x38, 0x22]);
        assert_eq!(d.bytes().unwrap(), &[]);
        assert_eq!(d.bytes().unwrap(), &[1, 2, 3]);
    }

    #[test]
    fn rejects_nested_values_without_traversing_them() {
        for name in [
            "module_id",
            "digest",
            "timestamp",
            "pcrs",
            "certificate",
            "cabundle",
        ] {
            let mut payload = encoded_value(|e| {
                e.map(1).unwrap().str(name).unwrap();
            });
            let value_start = payload.len();
            payload.extend_from_slice(&[0x81; 8_000]); // Wrong-type nested arrays, no terminal value.
            let mut decoder = minicbor_serde::Deserializer::new(&payload);
            assert!(AttestationDocument::deserialize(&mut decoder).is_err());
            assert!(
                decoder.decoder().position() <= value_start + 2,
                "{name} traversed the value"
            );
        }
        let mut protected = encoded_value(|e| {
            e.map(7_400).unwrap();
        });
        protected.extend_from_slice(&[0x81; 8_000]);
        assert!(validate_protected(&protected).is_err());
    }

    #[test]
    fn unprotected_headers_accept_only_empty_maps() {
        let payload = payload(false, false, false);
        for headers in [vec![0xa0], vec![0xbf, 0xff]] {
            let mut e = Encoder::new(Vec::new());
            e.array(4).unwrap().bytes(&[0xa1, 1, 0x38, 0x22]).unwrap();
            let mut input = e.into_writer();
            input.extend_from_slice(&headers);
            let mut e = Encoder::new(input);
            e.bytes(&payload).unwrap().bytes(&[0; 96]).unwrap();
            assert!(parse_attestation(&e.into_writer()).is_ok());
        }
        for input in [vec![0x80], vec![0xf6], vec![0x00], vec![0xd2, 0xa0]] {
            let mut decoder = minicbor_serde::Deserializer::new(&input);
            assert!(EmptyHeaders::deserialize(&mut decoder).is_err());
        }
        for indefinite in [false, true] {
            let mut e = Encoder::new(Vec::new());
            if indefinite {
                e.begin_map().unwrap();
            } else {
                e.map(1).unwrap();
            }
            e.str("ignored").unwrap();
            let mut input = e.into_writer();
            let value_start = input.len();
            input.extend_from_slice(&[0x81; 8_000]);
            let mut decoder = minicbor_serde::Deserializer::new(&input);
            assert!(EmptyHeaders::deserialize(&mut decoder).is_err());
            assert_eq!(decoder.decoder().position(), value_start);
        }
    }

    #[test]
    fn rejects_declared_wrong_envelope_lengths_and_nested_tags() {
        for header in [
            vec![0x85],
            vec![0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            vec![0xd2, 0xd8, 100, 0x84],
        ] {
            let mut input = header;
            input.extend_from_slice(&document(false, false, false)[1..]);
            assert!(parse_attestation(&input).is_err());
        }
        let mut input = vec![0x84, 0x44, 0xa1, 1, 0x38, 0x22, 0xa1];
        input.extend_from_slice(&[0x81; 8_000]); // Nonempty unprotected map, without even a key.
        assert!(parse_attestation(&input).is_err());
    }

    #[test]
    fn rejects_tagged_payload_and_field_names() {
        let payload = encode_payload(&default_fields());
        let mut tagged = vec![0xd8, 100];
        tagged.extend_from_slice(&payload);
        assert!(parse_attestation(&wrap_payload(false, false, &tagged)).is_err());
        let mut payload = payload;
        payload.splice(1..1, [0xd8, 100]);
        assert!(parse_attestation(&wrap_payload(false, false, &payload)).is_err());
    }

    #[test]
    fn collection_limits_stop_before_the_remaining_entries() {
        let mut bytes = encoded_value(|e| {
            e.map(1)
                .unwrap()
                .str("cabundle")
                .unwrap()
                .array(7_400)
                .unwrap();
            for _ in 0..=MAX_CA_BUNDLE {
                e.bytes(&[0x30]).unwrap();
            }
        });
        let end = bytes.len();
        bytes.extend_from_slice(&[0x81; 8_000]);
        let mut decoder = minicbor_serde::Deserializer::new(&bytes);
        assert!(AttestationDocument::deserialize(&mut decoder).is_err());
        assert_eq!(decoder.decoder().position(), end);

        let mut bytes = encoded_value(|e| {
            e.map(1).unwrap().str("pcrs").unwrap().map(250).unwrap();
            for _ in 0..2 {
                e.u8(0).unwrap().bytes(&[0; 48]).unwrap();
            }
        });
        let end = bytes.len();
        bytes.extend_from_slice(&[0x81; 8_000]);
        let mut decoder = minicbor_serde::Deserializer::new(&bytes);
        assert!(AttestationDocument::deserialize(&mut decoder).is_err());
        assert_eq!(decoder.decoder().position(), end);
    }

    #[test]
    fn payload_byte_fields_borrow_the_input() {
        let bytes = encode_payload(&default_fields());
        let parsed: AttestationDocument<'_> =
            decode_exact(Decoder::new(&bytes), FormatError::InvalidPayload).unwrap();
        let input = bytes.as_ptr_range();
        for value in core::iter::once(parsed.certificate)
            .chain(parsed.cabundle.iter().copied())
            .chain(parsed.pcrs.values().copied())
        {
            assert!(input.contains(&value.as_ptr()));
            assert!(value.as_ptr_range().end <= input.end);
        }
    }
}
