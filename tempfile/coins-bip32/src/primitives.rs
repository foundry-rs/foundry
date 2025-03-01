use crate::Bip32Error;
use coins_core::ser::ByteFormat;
use std::io::{Read, Write};

/// We treat the bip32 xpub bip49 ypub and bip84 zpub convention as a hint regarding address type.
/// Downstream crates are free to follow or ignore these hints when generating addresses from
/// extended keys.
#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub enum Hint {
    /// Standard Bip32 hint
    Legacy,
    /// Bip32 + Bip49 hint for Witness-via-P2SH
    Compatibility,
    /// Bip32 + Bip84 hint for Native SegWit
    SegWit,
}

/// A 4-byte key fingerprint
#[derive(Eq, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct KeyFingerprint(pub [u8; 4]);

impl From<[u8; 4]> for KeyFingerprint {
    fn from(v: [u8; 4]) -> Self {
        Self(v)
    }
}

impl ByteFormat for KeyFingerprint {
    type Error = Bip32Error;

    fn serialized_length(&self) -> usize {
        4
    }

    fn read_from<R>(reader: &mut R) -> Result<Self, Self::Error>
    where
        R: Read,
        Self: std::marker::Sized,
    {
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf)?;
        Ok(Self(buf))
    }

    fn write_to<W>(&self, writer: &mut W) -> Result<usize, Self::Error>
    where
        W: Write,
    {
        Ok(writer.write(&self.0)?)
    }
}

impl KeyFingerprint {
    /// Determines if the slice represents the same key fingerprint
    pub fn eq_slice(self, other: &[u8]) -> bool {
        self.0 == other
    }
}

impl std::fmt::Debug for KeyFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("KeyFingerprint {:x?}", self.0))
    }
}

/// A 32-byte chain code
#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub struct ChainCode(pub [u8; 32]);

impl From<[u8; 32]> for ChainCode {
    fn from(v: [u8; 32]) -> Self {
        Self(v)
    }
}

/// Info associated with an extended key
#[derive(Copy, Clone, Debug)]
pub struct XKeyInfo {
    /// The key depth in the HD tree
    pub depth: u8,
    /// The 4-byte Fingerprint of the parent
    pub parent: KeyFingerprint,
    /// The 4-byte derivation index of the key. If the most-significant byte is set, this key is
    /// hardened
    pub index: u32,
    /// The 32-byte chain code used to generate child keys
    pub chain_code: ChainCode,
    /// The key's stanadard output type preference
    pub hint: Hint,
}

impl PartialEq for XKeyInfo {
    fn eq(&self, other: &XKeyInfo) -> bool {
        self.depth == other.depth
            && self.parent == other.parent
            && self.index == other.index
            && self.chain_code == other.chain_code
    }
}
