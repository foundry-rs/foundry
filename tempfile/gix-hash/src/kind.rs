use std::str::FromStr;

use crate::{oid, Kind, ObjectId};

impl TryFrom<u8> for Kind {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Ok(match value {
            1 => Kind::Sha1,
            unknown => return Err(unknown),
        })
    }
}

impl FromStr for Kind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "sha1" | "SHA1" => Kind::Sha1,
            other => return Err(other.into()),
        })
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Kind::Sha1 => f.write_str("SHA1"),
        }
    }
}

impl Kind {
    /// Returns the shortest hash we support.
    #[inline]
    pub const fn shortest() -> Self {
        Self::Sha1
    }

    /// Returns the longest hash we support.
    #[inline]
    pub const fn longest() -> Self {
        Self::Sha1
    }

    /// Returns a buffer suitable to hold the longest possible hash in hex.
    #[inline]
    pub const fn hex_buf() -> [u8; Kind::longest().len_in_hex()] {
        [0u8; Kind::longest().len_in_hex()]
    }

    /// Returns a buffer suitable to hold the longest possible hash as raw bytes.
    #[inline]
    pub const fn buf() -> [u8; Kind::longest().len_in_bytes()] {
        [0u8; Kind::longest().len_in_bytes()]
    }

    /// Returns the amount of bytes needed to encode this instance as hexadecimal characters.
    #[inline]
    pub const fn len_in_hex(&self) -> usize {
        match self {
            Kind::Sha1 => 40,
        }
    }
    /// Returns the amount of bytes taken up by the hash of this instance.
    #[inline]
    pub const fn len_in_bytes(&self) -> usize {
        match self {
            Kind::Sha1 => 20,
        }
    }

    /// Returns the kind of hash that would fit the given `hex_len`, or `None` if there is no fitting hash.
    /// Note that `0` as `hex_len` up to 40 always yields `Sha1`.
    #[inline]
    pub const fn from_hex_len(hex_len: usize) -> Option<Self> {
        Some(match hex_len {
            0..=40 => Kind::Sha1,
            _ => return None,
        })
    }

    /// Converts a size in bytes as obtained by `Kind::len_in_bytes()` into the corresponding hash kind, if possible.
    ///
    /// **Panics** if the hash length doesn't match a known hash.
    ///
    /// NOTE that this method isn't public as it shouldn't be encouraged to assume all hashes have the same length.
    /// However, if there should be such a thing, our `oid` implementation will have to become an enum and it's pretty breaking
    /// to the way it's currently being used as auto-dereffing doesn't work anymore. Let's hope it won't happen.
    // TODO: make 'const' once Rust 1.57 is more readily available in projects using 'gitoxide'.
    #[inline]
    pub(crate) fn from_len_in_bytes(bytes: usize) -> Self {
        match bytes {
            20 => Kind::Sha1,
            _ => panic!("BUG: must be called only with valid hash lengths produced by len_in_bytes()"),
        }
    }

    /// Create a shared null-id of our hash kind.
    #[inline]
    pub fn null_ref(&self) -> &'static oid {
        match self {
            Kind::Sha1 => oid::null_sha1(),
        }
    }

    /// Create an owned null-id of our hash kind.
    #[inline]
    pub const fn null(&self) -> ObjectId {
        match self {
            Kind::Sha1 => ObjectId::null_sha1(),
        }
    }
}
