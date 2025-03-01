//! Contains simplified access to `bech32` and `base58check` encoder/decoder for Bitcoin
//! addresses. Also defines common encoder errors.

use bech32::{
    decode as b32_decode, encode as b32_encode, u5, Error as BechError, FromBase32, ToBase32,
};

use bs58::{
    decode as bs58_decode, decode::Error as Bs58DecodeError, encode as bs58_encode,
    encode::Error as Bs58EncodeError,
};

use thiserror::Error;

/// Errors that can be returned by the Bitcoin `AddressEncoder`.
#[derive(Debug, Error)]
pub enum EncodingError {
    /// Returned when ScriptPubkey type is unknown. May be non-standard or newer than lib version.
    #[error("Non-standard ScriptPubkey type")]
    UnknownScriptType,

    /// Bech32 HRP does not match the current network.
    #[error("Bech32 HRP does not match. \nGot {:?} expected {:?} Hint: Is this address for another network?", got, expected)]
    WrongHrp {
        /// The actual HRP.
        got: String,
        /// The expected HRP.
        expected: String,
    },

    /// Base58Check version does not match the current network
    #[error("Base58Check version does not match. \nGot {:?} expected {:?} Hint: Is this address for another network?", got, expected)]
    WrongVersion {
        /// The actual version byte.
        got: u8,
        /// The expected version byte.
        expected: u8,
    },

    /// Bubbled up error from base58check library
    #[error("{0}")]
    Bs58Decode(#[from] Bs58DecodeError),

    /// Bubbled up error from base58check library
    #[error("{0}")]
    Bs58Encode(#[from] Bs58EncodeError),

    /// Bubbled up error from bech32 library
    #[error(transparent)]
    BechError(#[from] BechError),

    /// Op Return ScriptPubkey was passed to encoder
    #[error("Can't encode op return scripts as addresses")]
    NullDataScript,

    /// Invalid Segwit Version
    #[error("Invalid Segwit Version: {0}")]
    SegwitVersionError(u8),

    /// Invalid Address Size
    #[error("Invalid Address Size")]
    InvalidSizeError,
}

/// A simple result type alias
pub type EncodingResult<T> = Result<T, EncodingError>;

/// Encode a byte vector to bech32. This function expects `v` to be a witness program, and will
/// return an `UnknownScriptType` if it does not meet the witness program format.
pub fn encode_bech32(hrp: &str, v: u8, h: &[u8]) -> EncodingResult<String> {
    let mut v = vec![u5::try_from_u8(v)?];
    v.extend(&h.to_base32());
    b32_encode(hrp, &v, bech32::Variant::Bech32).map_err(|v| v.into())
}

/// Decode a witness program from a bech32 string. Caller specifies an expected HRP. If a
/// different HRP is found, returns `WrongHrp`.
pub fn decode_bech32(expected_hrp: &str, s: &str) -> EncodingResult<(u8, Vec<u8>)> {
    let (hrp, data, _variant) = b32_decode(s)?;
    if hrp != expected_hrp {
        return Err(EncodingError::WrongHrp {
            got: hrp,
            expected: expected_hrp.to_owned(),
        });
    }

    // Extract the witness version and payload
    let (v, p) = data.split_at(1);
    let payload = Vec::from_base32(p)?;

    Ok((v[0].to_u8(), payload))
}

/// Encodes a byte slice to base58check with the specified version byte.
pub fn encode_base58(v: &[u8]) -> String {
    bs58_encode(v).with_check().into_string()
}

/// Decodes base58check into a byte string. Returns a
/// `EncodingError::Bs58Decode` if unsuccesful
pub fn decode_base58(expected_prefix: u8, s: &str) -> EncodingResult<Vec<u8>> {
    let res = bs58_decode(s).with_check(None).into_vec()?;

    if let Some(version) = res.first() {
        if version != &expected_prefix {
            return Err(EncodingError::Bs58Decode(Bs58DecodeError::InvalidVersion {
                ver: *version,
                expected_ver: expected_prefix,
            }));
        }
    }

    Ok(res)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn it_should_encode_and_decode_arbitrary_bech32() {
        let cases = [
            // Lightning invoice
            ("lnbc20m", "lnbc20m1pvjluezpp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqhp58yjmdan79s6qqdhdzgynm4zwqd5d7xmw5fk98klysy043l2ahrqscc6gd6ql3jrc5yzme8v4ntcewwz5cnw92tz0pc8qcuufvq7khhr8wpald05e92xw006sq94mg8v2ndf4sefvf9sygkshp5zfem29trqq2yxxz7"),
            // Namecoin address
            ("nc", "nc1qanwztr5zvd309vjf9ks9c2c3hyw3sqpppwkuut"),
            // Handshake address
            ("hs", "hs1q8vn02tnktq3tmztny8nysel6vtkuuy9k0whtty"),
            // Random data
            ("ab", "ab1qm7dpnrqefvf4ee67"),
            ("lol", "lol1yrtmpa4p98nerppeu3h00my48ejmmyj629aeyqhur7wfrzfwqj99v875saeetusxtphs3q2"),
        ];

        for case in cases.iter() {
            let (version, data) = decode_bech32(case.0, case.1).unwrap();
            let reencoded = encode_bech32(case.0, version, &data).unwrap();
            assert_eq!(case.1, reencoded);
        }
    }

    #[test]
    fn it_should_encode_and_decode_base58_pkh() {
        let version = 0x00;
        let addrs = [
            "1AqE7oGF1EUoJviX1uuYrwpRBdEBTuGhES",
            "1J2kECACFMDPyYjCBddKYbtzJMc6kv5FbA",
            "1ADKfX19iy3EFUoG5qGLSHNXb4c1SSHFNF",
            "12cKuAyj2jmrmMPBMtoeAt47DrJ5WRK2R5",
            "19R4yak7BGX8fcWNvtuuTSjQGC43U4qadJ",
            "1MT3dyC8YgEGY37yPwPtnvyau8HjGiMhhM",
            "1NDyJtNTjmwk5xPNhjgAMu4HDHigtobu1s",
            "1HMPBDt3HAD6o3zAxotBCS9o8KqCuYoapF",
            "16o4roRP8dapRJraVNnw99xBh3J1Wkk5m8",
        ];
        for addr in addrs.iter() {
            let s = decode_base58(version, addr).unwrap();
            let reencoded = encode_base58(&s);
            assert_eq!(*addr, reencoded);
        }
    }

    #[test]
    fn it_should_encode_and_decode_base58_sh() {
        let version = 0x05;
        let addrs = [
            "3HXNFmJpxjgTVFN35Y9f6Waje5YFsLEQZ2",
            "35mpC7r8fGrt2WTBTkQ56xBgm1k1QCY9CQ",
            "345KNsztA2frN7V2TTZ2a9Vt6ojH8VSXFM",
            "37QxcQb7U549M1QoDpXuRZMcTjRF52mfjx",
            "377mKFYsaJPsxYSB5aFfx8SW3RaN5BzZVh",
            "3GPM5uAPoqJ4CAst3GiraHPGFxSin6Ch2b",
            "3LVq5zEBW48DjrqtmExR1YYDfJLmp8ryQE",
            "3GfrmGENZFbV4rMWUxUxeo2yUnEnSDQ5BP",
            "372sRbqCNQ1xboWCcc7XSbjptv8pzF9sBq",
        ];
        for addr in addrs.iter() {
            let s = decode_base58(version, addr).unwrap();
            let reencoded = encode_base58(&s);
            assert_eq!(*addr, reencoded);
        }
    }

    #[test]
    fn it_should_error_on_wrong_version_and_hrp_and_invalid_addrs() {
        match decode_bech32("tb", "bc1q233q49ve8ysdsztqh9ue57m6227627j8ztscl9") {
            Ok(_) => panic!("expected an error"),
            Err(EncodingError::WrongHrp {
                got: _,
                expected: _,
            }) => {}
            _ => panic!("Got the wrong error"),
        }
        match decode_base58(1, "3HXNFmJpxjgTVFN35Y9f6Waje5YFsLEQZ2") {
            Ok(_) => panic!("expected an error"),
            Err(EncodingError::Bs58Decode(Bs58DecodeError::InvalidVersion {
                ver: 5,
                expected_ver: 1,
            })) => {}
            _ => panic!("Got the wrong error"),
        }
        match decode_bech32("bc", "bc1qqh9ue57m6227627j8ztscl9") {
            Ok(_) => panic!("expected an error"),
            Err(EncodingError::BechError(_)) => {}
            _ => panic!("Got the wrong error"),
        }
        match decode_base58(5, "3HXNf6Waje5YFsLEQZ2") {
            Ok(_) => panic!("expected an error"),
            Err(EncodingError::Bs58Decode(_)) => {}
            _ => panic!("Got the wrong error"),
        }
    }
}
