//! JWT (JSON Web Token) utilities for the Engine API.

use alloc::string::String;
use alloy_primitives::hex;
use core::{str::FromStr, time::Duration};
use jsonwebtoken::get_current_timestamp;
use rand::Rng;

#[cfg(feature = "std")]
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[cfg(feature = "serde")]
use jsonwebtoken::{errors::ErrorKind, Algorithm, DecodingKey, Validation};

/// Errors returned by the [`JwtSecret`]
#[derive(Debug, derive_more::Display)]
pub enum JwtError {
    /// An error encountered while decoding the hexadecimal string for the JWT secret.
    #[display("{_0}")]
    JwtSecretHexDecodeError(hex::FromHexError),

    /// The JWT key length provided is invalid, expecting a specific length.
    #[display("JWT key is expected to have a length of {_0} digits. {_1} digits key provided")]
    InvalidLength(usize, usize),

    /// The signature algorithm used in the JWT is not supported. Only HS256 is supported.
    #[display("unsupported signature algorithm. Only HS256 is supported")]
    UnsupportedSignatureAlgorithm,

    /// The provided signature in the JWT is invalid.
    #[display("provided signature is invalid")]
    InvalidSignature,

    /// The "iat" (issued-at) claim in the JWT is not within the allowed ±60 seconds from the
    /// current time.
    #[display("IAT (issued-at) claim is not within ±60 seconds from the current time")]
    InvalidIssuanceTimestamp,

    /// The Authorization header is missing or invalid in the context of JWT validation.
    #[display("Authorization header is missing or invalid")]
    MissingOrInvalidAuthorizationHeader,

    /// An error occurred during JWT decoding.
    #[display("JWT decoding error: {_0}")]
    JwtDecodingError(String),

    /// An error occurred while creating a directory to store the JWT.
    #[display("failed to create dir {path:?}: {source}")]
    #[cfg(feature = "std")]
    CreateDir {
        /// The source `io::Error`.
        source: io::Error,
        /// The path related to the operation.
        path: PathBuf,
    },

    /// An error occurred while reading the JWT from a file.
    #[display("failed to read from {path:?}: {source}")]
    #[cfg(feature = "std")]
    Read {
        /// The source `io::Error`.
        source: io::Error,
        /// The path related to the operation.
        path: PathBuf,
    },

    /// An error occurred while writing the JWT to a file.
    #[display("failed to write to {path:?}: {source}")]
    #[cfg(feature = "std")]
    Write {
        /// The source `io::Error`.
        source: io::Error,
        /// The path related to the operation.
        path: PathBuf,
    },
}

impl From<hex::FromHexError> for JwtError {
    fn from(err: hex::FromHexError) -> Self {
        Self::JwtSecretHexDecodeError(err)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for JwtError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::JwtSecretHexDecodeError(err) => Some(err),
            Self::CreateDir { source, .. }
            | Self::Read { source, .. }
            | Self::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Length of the hex-encoded 256 bit secret key.
/// A 256-bit encoded string in Rust has a length of 64 digits because each digit represents 4 bits
/// of data. In hexadecimal representation, each digit can have 16 possible values (0-9 and A-F), so
/// 4 bits can be represented using a single hex digit. Therefore, to represent a 256-bit string,
/// we need 64 hexadecimal digits (256 bits ÷ 4 bits per digit = 64 digits).
const JWT_SECRET_LEN: usize = 64;

/// The JWT `iat` (issued-at) claim cannot exceed +-60 seconds from the current time.
const JWT_MAX_IAT_DIFF: Duration = Duration::from_secs(60);

/// The execution layer client MUST support at least the following alg HMAC + SHA256 (HS256)
#[cfg(feature = "serde")]
const JWT_SIGNATURE_ALGO: Algorithm = Algorithm::HS256;

/// Claims in JWT are used to represent a set of information about an entity.
///
/// Claims are essentially key-value pairs that are encoded as JSON objects and included in the
/// payload of a JWT. They are used to transmit information such as the identity of the entity, the
/// time the JWT was issued, and the expiration time of the JWT, among others.
///
/// The Engine API spec requires that just the `iat` (issued-at) claim is provided.
/// It ignores claims that are optional or additional for this specification.
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Claims {
    /// The "iat" value MUST be a number containing a NumericDate value.
    /// According to the RFC A NumericDate represents the number of seconds since
    /// the UNIX_EPOCH.
    /// - [`RFC-7519 - Spec`](https://www.rfc-editor.org/rfc/rfc7519#section-4.1.6)
    /// - [`RFC-7519 - Notations`](https://www.rfc-editor.org/rfc/rfc7519#section-2)
    pub iat: u64,
    /// The "exp" (expiration time) claim identifies the expiration time on or after which the JWT
    /// MUST NOT be accepted for processing.
    pub exp: Option<u64>,
}

impl Claims {
    /// Creates a new instance of [`Claims`] with the current timestamp as the `iat` claim.
    pub fn with_current_timestamp() -> Self {
        Self { iat: get_current_timestamp(), exp: None }
    }

    /// Checks if the `iat` claim is within the allowed range from the current time.
    pub fn is_within_time_window(&self) -> bool {
        let now_secs = get_current_timestamp();
        now_secs.abs_diff(self.iat) <= JWT_MAX_IAT_DIFF.as_secs()
    }
}

impl Default for Claims {
    /// By default, the `iat` claim is set to the current timestamp.
    fn default() -> Self {
        Self::with_current_timestamp()
    }
}

/// Value-object holding a reference to a hex-encoded 256-bit secret key.
///
/// A JWT secret key is used to secure JWT-based authentication. The secret key is
/// a shared secret between the server and the client and is used to calculate a digital signature
/// for the JWT, which is included in the JWT along with its payload.
///
/// See also: [Secret key - Engine API specs](https://github.com/ethereum/execution-apis/blob/main/src/engine/authentication.md#key-distribution)
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct JwtSecret([u8; 32]);

impl JwtSecret {
    /// Creates an instance of [`JwtSecret`].
    ///
    /// Returns an error if one of the following applies:
    /// - `hex` is not a valid hexadecimal string
    /// - `hex` argument length is less than `JWT_SECRET_LEN`
    ///
    /// This strips the leading `0x`, if any.
    pub fn from_hex<S: AsRef<str>>(hex: S) -> Result<Self, JwtError> {
        let hex = hex.as_ref().trim().trim_start_matches("0x");
        if hex.len() == JWT_SECRET_LEN {
            let hex_bytes = hex::decode(hex)?;
            // is 32bytes, see length check
            let bytes = hex_bytes.try_into().expect("is expected len");
            Ok(Self(bytes))
        } else {
            Err(JwtError::InvalidLength(JWT_SECRET_LEN, hex.len()))
        }
    }

    /// Tries to load a [`JwtSecret`] from the specified file path.
    /// I/O or secret validation errors might occur during read operations in the form of
    /// a [`JwtError`].
    #[cfg(feature = "std")]
    pub fn from_file(fpath: &Path) -> Result<Self, JwtError> {
        fs::read_to_string(fpath)
            .map_err(|err| JwtError::Read { source: err, path: fpath.into() })
            .and_then(Self::from_hex)
    }

    /// Creates a random [`JwtSecret`] and tries to store it at the specified path. I/O errors might
    /// occur during write operations in the form of a [`JwtError`]
    #[cfg(feature = "std")]
    pub fn try_create_random(fpath: &Path) -> Result<Self, JwtError> {
        if let Some(dir) = fpath.parent() {
            // Create parent directory
            fs::create_dir_all(dir)
                .map_err(|err| JwtError::CreateDir { source: err, path: fpath.into() })?
        }

        let secret = Self::random();
        let bytes = &secret.0;
        let hex = hex::encode(bytes);
        fs::write(fpath, hex).map_err(|err| JwtError::Write { source: err, path: fpath.into() })?;
        Ok(secret)
    }

    /// Validates a JWT token along the following rules:
    /// - The JWT signature is valid.
    /// - The JWT is signed with the `HMAC + SHA256 (HS256)` algorithm.
    /// - The JWT `iat` (issued-at) claim is a timestamp within +-60 seconds from the current time.
    /// - The JWT `exp` (expiration time) claim is validated by default if defined.
    ///
    /// See also: [JWT Claims - Engine API specs](https://github.com/ethereum/execution-apis/blob/main/src/engine/authentication.md#jwt-claims)
    #[cfg(feature = "serde")]
    pub fn validate(&self, jwt: &str) -> Result<(), JwtError> {
        // Create a new validation object with the required signature algorithm
        // and ensure that the `iat` claim is present. The `exp` claim is validated if defined.
        let mut validation = Validation::new(JWT_SIGNATURE_ALGO);
        validation.set_required_spec_claims(&["iat"]);
        let bytes = &self.0;

        match jsonwebtoken::decode::<Claims>(jwt, &DecodingKey::from_secret(bytes), &validation) {
            Ok(token) => {
                if !token.claims.is_within_time_window() {
                    Err(JwtError::InvalidIssuanceTimestamp)?
                }
            }
            Err(err) => match *err.kind() {
                ErrorKind::InvalidSignature => Err(JwtError::InvalidSignature)?,
                ErrorKind::InvalidAlgorithm => Err(JwtError::UnsupportedSignatureAlgorithm)?,
                _ => {
                    let detail = format!("{err}");
                    Err(JwtError::JwtDecodingError(detail))?
                }
            },
        };

        Ok(())
    }

    /// Generates a random [`JwtSecret`] containing a hex-encoded 256 bit secret key.
    pub fn random() -> Self {
        let random_bytes: [u8; 32] = rand::thread_rng().gen();
        let secret = hex::encode(random_bytes);
        Self::from_hex(secret).unwrap()
    }

    /// Encode the header and claims given and sign the payload using the algorithm from the header
    /// and the key.
    #[cfg(feature = "serde")]
    pub fn encode(&self, claims: &Claims) -> Result<String, jsonwebtoken::errors::Error> {
        let bytes = &self.0;
        let key = jsonwebtoken::EncodingKey::from_secret(bytes);
        let algo = jsonwebtoken::Header::new(Algorithm::HS256);
        jsonwebtoken::encode(&algo, claims, &key)
    }

    /// Returns the secret key as a byte slice.
    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl core::fmt::Debug for JwtSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("JwtSecretHash").field(&"{{}}").finish()
    }
}

impl FromStr for JwtSecret {
    type Err = JwtError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use similar_asserts::assert_eq;
    #[cfg(feature = "std")]
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;

    #[test]
    fn from_hex() {
        let key = "f79ae8046bc11c9927afe911db7143c51a806c4a537cc08e0d37140b0192f430";
        let secret: Result<JwtSecret, _> = JwtSecret::from_hex(key);
        assert!(secret.is_ok());

        let secret: Result<JwtSecret, _> = JwtSecret::from_hex(key);
        assert!(secret.is_ok());
    }

    #[test]
    fn original_key_integrity_across_transformations() {
        let original = "f79ae8046bc11c9927afe911db7143c51a806c4a537cc08e0d37140b0192f430";
        let secret = JwtSecret::from_hex(original).unwrap();
        let bytes = &secret.0;
        let computed = hex::encode(bytes);
        assert_eq!(original, computed);
    }

    #[test]
    fn secret_has_64_hex_digits() {
        let expected_len = 64;
        let secret = JwtSecret::random();
        let hex = hex::encode(secret.0);
        assert_eq!(hex.len(), expected_len);
    }

    #[test]
    fn creation_ok_hex_string_with_0x() {
        let hex: String =
            "0x7365637265747365637265747365637265747365637265747365637265747365".into();
        let result = JwtSecret::from_hex(hex);
        assert!(result.is_ok());
    }

    #[test]
    fn creation_error_wrong_len() {
        let hex = "f79ae8046";
        let result = JwtSecret::from_hex(hex);
        assert!(matches!(result, Err(JwtError::InvalidLength(_, _))));
    }

    #[test]
    fn creation_error_wrong_hex_string() {
        let hex: String = "This__________Is__________Not_______An____Hex_____________String".into();
        let result = JwtSecret::from_hex(hex);
        assert!(matches!(result, Err(JwtError::JwtSecretHexDecodeError(_))));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn validation_ok() {
        let secret = JwtSecret::random();
        let claims = Claims { iat: get_current_timestamp(), exp: Some(10000000000) };
        let jwt = secret.encode(&claims).unwrap();

        let result = secret.validate(&jwt);

        assert!(matches!(result, Ok(())));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn validation_with_current_time_ok() {
        let secret = JwtSecret::random();
        let claims = Claims::default();
        let jwt = secret.encode(&claims).unwrap();

        let result = secret.validate(&jwt);

        assert!(matches!(result, Ok(())));
    }

    #[test]
    #[cfg(all(feature = "std", feature = "serde"))]
    fn validation_error_iat_out_of_window() {
        let secret = JwtSecret::random();

        // Check past 'iat' claim more than 60 secs
        let offset = Duration::from_secs(JWT_MAX_IAT_DIFF.as_secs() + 1);
        let out_of_window_time = SystemTime::now().checked_sub(offset).unwrap();
        let claims = Claims { iat: to_u64(out_of_window_time), exp: Some(10000000000) };
        let jwt = secret.encode(&claims).unwrap();

        let result = secret.validate(&jwt);

        assert!(matches!(result, Err(JwtError::InvalidIssuanceTimestamp)));

        // Check future 'iat' claim more than 60 secs
        let offset = Duration::from_secs(JWT_MAX_IAT_DIFF.as_secs() + 1);
        let out_of_window_time = SystemTime::now().checked_add(offset).unwrap();
        let claims = Claims { iat: to_u64(out_of_window_time), exp: Some(10000000000) };
        let jwt = secret.encode(&claims).unwrap();

        let result = secret.validate(&jwt);

        assert!(matches!(result, Err(JwtError::InvalidIssuanceTimestamp)));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn validation_error_exp_expired() {
        let secret = JwtSecret::random();
        let claims = Claims { iat: get_current_timestamp(), exp: Some(1) };
        let jwt = secret.encode(&claims).unwrap();

        let result = secret.validate(&jwt);

        assert!(matches!(result, Err(JwtError::JwtDecodingError(_))));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn validation_error_wrong_signature() {
        let secret_1 = JwtSecret::random();
        let claims = Claims { iat: get_current_timestamp(), exp: Some(10000000000) };
        let jwt = secret_1.encode(&claims).unwrap();

        // A different secret will generate a different signature.
        let secret_2 = JwtSecret::random();
        let result = secret_2.validate(&jwt);
        assert!(matches!(result, Err(JwtError::InvalidSignature)));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn validation_error_unsupported_algorithm() {
        let secret = JwtSecret::random();
        let bytes = &secret.0;

        let key = EncodingKey::from_secret(bytes);
        let unsupported_algo = Header::new(Algorithm::HS384);

        let claims = Claims { iat: get_current_timestamp(), exp: Some(10000000000) };
        let jwt = encode(&unsupported_algo, &claims, &key).unwrap();
        let result = secret.validate(&jwt);

        assert!(matches!(result, Err(JwtError::UnsupportedSignatureAlgorithm)));
    }

    #[test]
    #[cfg(feature = "serde")]
    fn valid_without_exp_claim() {
        let secret = JwtSecret::random();

        let claims = Claims { iat: get_current_timestamp(), exp: None };
        let jwt = secret.encode(&claims).unwrap();

        let result = secret.validate(&jwt);

        assert!(matches!(result, Ok(())));
    }

    #[test]
    #[cfg(feature = "std")]
    fn ephemeral_secret_created() {
        let fpath: &Path = Path::new("secret0.hex");
        assert!(fs::metadata(fpath).is_err());
        JwtSecret::try_create_random(fpath).expect("A secret file should be created");
        assert!(fs::metadata(fpath).is_ok());
        fs::remove_file(fpath).unwrap();
    }

    #[test]
    #[cfg(feature = "std")]
    fn valid_secret_provided() {
        let fpath = Path::new("secret1.hex");
        assert!(fs::metadata(fpath).is_err());

        let secret = JwtSecret::random();
        fs::write(fpath, hex(&secret)).unwrap();

        match JwtSecret::from_file(fpath) {
            Ok(gen_secret) => {
                fs::remove_file(fpath).unwrap();
                assert_eq!(hex(&gen_secret), hex(&secret));
            }
            Err(_) => {
                fs::remove_file(fpath).unwrap();
            }
        }
    }

    #[test]
    #[cfg(feature = "std")]
    fn invalid_hex_provided() {
        let fpath = Path::new("secret2.hex");
        fs::write(fpath, "invalid hex").unwrap();
        let result = JwtSecret::from_file(fpath);
        assert!(result.is_err());
        fs::remove_file(fpath).unwrap();
    }

    #[test]
    #[cfg(feature = "std")]
    fn provided_file_not_exists() {
        let fpath = Path::new("secret3.hex");
        let result = JwtSecret::from_file(fpath);
        assert_matches!(result, Err(JwtError::Read {source: _,path }) if path == fpath.to_path_buf());
        assert!(fs::metadata(fpath).is_err());
    }

    #[test]
    #[cfg(feature = "std")]
    fn provided_file_is_a_directory() {
        let dir = tempdir().unwrap();
        let result = JwtSecret::from_file(dir.path());
        assert_matches!(result, Err(JwtError::Read {source: _,path}) if path == dir.into_path());
    }

    #[cfg(feature = "std")]
    fn to_u64(time: SystemTime) -> u64 {
        time.duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    fn hex(secret: &JwtSecret) -> String {
        hex::encode(secret.0)
    }
}
