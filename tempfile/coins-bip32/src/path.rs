use std::{
    convert::TryFrom,
    io::{Read, Write},
    iter::{FromIterator, IntoIterator},
    slice::Iter,
    str::FromStr,
};

use coins_core::ser::ByteFormat;

use crate::{primitives::KeyFingerprint, Bip32Error, BIP32_HARDEN};

fn try_parse_index(s: &str) -> Result<u32, Bip32Error> {
    let mut index_str = s.to_owned();
    let harden = if s.ends_with('\'') || s.ends_with('h') {
        index_str.pop();
        true
    } else {
        false
    };

    index_str
        .parse::<u32>()
        .map(|v| if harden { harden_index(v) } else { v })
        .map_err(|_| Bip32Error::MalformattedDerivation(s.to_owned()))
}

fn encode_index(idx: u32, harden: char) -> String {
    let mut s = (idx % BIP32_HARDEN).to_string();
    if idx >= BIP32_HARDEN {
        s.push(harden);
    }
    s
}

/// Converts an raw index to hardened
pub const fn harden_index(index: u32) -> u32 {
    index + BIP32_HARDEN
}

/// A Bip32 derivation path
#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub struct DerivationPath(Vec<u32>);

impl serde::Serialize for DerivationPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.derivation_string())
    }
}

impl<'de> serde::Deserialize<'de> for DerivationPath {
    fn deserialize<D>(deserializer: D) -> Result<DerivationPath, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: &str = serde::Deserialize::deserialize(deserializer)?;
        s.parse::<DerivationPath>()
            .map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

impl DerivationPath {
    #[doc(hidden)]
    pub fn custom_string(&self, root: &str, joiner: char, harden: char) -> String {
        std::iter::once(root.to_owned())
            .chain(self.0.iter().map(|s| encode_index(*s, harden)))
            .collect::<Vec<String>>()
            .join(&joiner.to_string())
    }

    /// Return the last index in the path. None if the path is the root.
    pub fn last(&self) -> Option<&u32> {
        self.0.last()
    }

    /// Converts the path to a standard bip32 string. e.g `"m/44'/0'/0/32"`.
    pub fn derivation_string(&self) -> String {
        self.custom_string("m", '/', '\'')
    }

    /// Returns `True` if there are no indices in the path
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The number of derivations in the path
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Make an iterator over the path indices
    pub fn iter(&self) -> Iter<u32> {
        self.0.iter()
    }

    /// `true` if `other` is a prefix of `self`
    pub fn starts_with(&self, other: &Self) -> bool {
        self.0.starts_with(&other.0)
    }

    /// Remove a prefix from a derivation. Return a new DerivationPath without the prefix.
    /// This is useful for determining the path to rech some descendant from some ancestor.
    pub fn without_prefix(&self, prefix: &Self) -> Option<DerivationPath> {
        if !self.starts_with(prefix) {
            None
        } else {
            Some(self.0[prefix.len()..].to_vec().into())
        }
    }

    /// Convenience function for finding the last hardened derivation in a path.
    /// Returns the index and the element. If there is no hardened derivation, it
    /// will return (0, None).
    pub fn last_hardened(&self) -> (usize, Option<u32>) {
        match self.iter().rev().position(|v| *v >= BIP32_HARDEN) {
            Some(rev_pos) => {
                let pos = self.len() - rev_pos - 1;
                (pos, Some(self.0[pos]))
            }
            None => (0, None),
        }
    }

    /// Return a clone with a resized path. If the new size is shorter, this truncates it. If the
    /// new path is longer, we pad with the second argument.
    pub fn resized(&self, size: usize, pad_with: u32) -> Self {
        let mut child = self.clone();
        child.0.resize(size, pad_with);
        child
    }

    /// Append an additional derivation to the end, return a clone
    pub fn extended(&self, idx: u32) -> Self {
        let mut child = self.clone();
        child.0.push(idx);
        child
    }
}

impl From<&DerivationPath> for DerivationPath {
    fn from(v: &DerivationPath) -> Self {
        v.clone()
    }
}

impl From<Vec<u32>> for DerivationPath {
    fn from(v: Vec<u32>) -> Self {
        Self(v)
    }
}

impl From<&Vec<u32>> for DerivationPath {
    fn from(v: &Vec<u32>) -> Self {
        Self(v.clone())
    }
}

impl From<&[u32]> for DerivationPath {
    fn from(v: &[u32]) -> Self {
        Self(Vec::from(v))
    }
}

impl TryFrom<u32> for DerivationPath {
    type Error = Bip32Error;

    fn try_from(v: u32) -> Result<Self, Self::Error> {
        Ok(Self(vec![v]))
    }
}

impl TryFrom<&str> for DerivationPath {
    type Error = Bip32Error;

    fn try_from(v: &str) -> Result<Self, Self::Error> {
        v.parse()
    }
}

impl FromIterator<u32> for DerivationPath {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = u32>,
    {
        Vec::from_iter(iter).into()
    }
}

impl FromStr for DerivationPath {
    type Err = Bip32Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split('/')
            .filter(|v| v != &"m")
            .map(try_parse_index)
            .collect::<Result<Vec<u32>, Bip32Error>>()
            .map(|v| v.into())
            .map_err(|_| Bip32Error::MalformattedDerivation(s.to_owned()))
    }
}

/// A Derivation Path for a bip32 key
#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeyDerivation {
    /// The root key fingerprint
    pub root: KeyFingerprint,
    /// The derivation path from the root key
    pub path: DerivationPath,
}

impl KeyDerivation {
    /// `true` if the keys share a root fingerprint, `false` otherwise. Note that on key
    /// fingerprints, which may collide accidentally, or be intentionally collided.
    pub fn same_root(&self, other: &Self) -> bool {
        self.root == other.root
    }

    /// `true` if this key is an ancestor of other, `false` otherwise. Note that on key
    /// fingerprints, which may collide accidentally, or be intentionally collided.
    pub fn is_possible_ancestor_of(&self, other: &Self) -> bool {
        self.same_root(other) && other.path.starts_with(&self.path)
    }

    /// Returns the path to the decendant.
    pub fn path_to_descendant(&self, descendant: &Self) -> Option<DerivationPath> {
        descendant.path.without_prefix(&self.path)
    }

    /// Return a clone with a resized path. If the new size is shorter, this truncates it. If the
    /// new path is longer, we pad with the second argument.
    pub fn resized(&self, size: usize, pad_with: u32) -> Self {
        Self {
            root: self.root,
            path: self.path.resized(size, pad_with),
        }
    }

    /// Append an additional derivation to the end, return a clone
    pub fn extended(&self, idx: u32) -> Self {
        Self {
            root: self.root,
            path: self.path.extended(idx),
        }
    }
}

impl ByteFormat for KeyDerivation {
    type Error = Bip32Error;

    fn serialized_length(&self) -> usize {
        4 + 4 * self.path.len()
    }

    fn read_from<T>(_reader: &mut T) -> Result<Self, Self::Error>
    where
        T: Read,
        Self: std::marker::Sized,
    {
        unimplemented!()
        // if limit == 0 {
        //     return Err(SerError::RequiresLimit.into());
        // }

        // if limit > 255 {
        //     return Err(Bip32Error::InvalidBip32Path);
        // }

        // let mut finger = [0u8; 4];
        // reader.read_exact(&mut finger)?;

        // let mut path = vec![];
        // for _ in 0..limit {
        //     let mut buf = [0u8; 4];
        //     reader.read_exact(&mut buf)?;
        //     path.push(u32::from_le_bytes(buf));
        // }

        // Ok(KeyDerivation {
        //     root: finger.into(),
        //     path: path.into(),
        // })
    }

    fn write_to<T>(&self, writer: &mut T) -> Result<usize, Self::Error>
    where
        T: Write,
    {
        let mut length = writer.write(&self.root.0)?;
        for i in self.path.iter() {
            length += writer.write(&i.to_le_bytes())?;
        }
        Ok(length)
    }
}

#[cfg(test)]
pub mod test {
    use super::*;

    #[test]
    fn it_parses_index_strings() {
        let cases = [("32", 32), ("32h", 32 + BIP32_HARDEN), ("0h", BIP32_HARDEN)];
        for case in cases.iter() {
            match try_parse_index(case.0) {
                Ok(v) => assert_eq!(v, case.1),
                Err(e) => panic!("unexpected error {}", e),
            }
        }
    }

    #[test]
    fn it_handles_malformatted_indices() {
        let cases = ["-", "h", "toast", "憂鬱"];
        for case in cases.iter() {
            match try_parse_index(case) {
                Ok(_) => panic!("expected an error"),
                Err(Bip32Error::MalformattedDerivation(e)) => assert_eq!(&e, case),
                Err(e) => panic!("unexpected error {}", e),
            }
        }
    }

    #[test]
    fn it_parses_derivation_strings() {
        let cases = [
            ("m/32", vec![32]),
            ("m/32'", vec![32 + BIP32_HARDEN]),
            ("m/0'/32/5/5/5", vec![BIP32_HARDEN, 32, 5, 5, 5]),
            ("32", vec![32]),
            ("32'", vec![32 + BIP32_HARDEN]),
            ("0'/32/5/5/5", vec![BIP32_HARDEN, 32, 5, 5, 5]),
        ];
        for case in cases.iter() {
            match case.0.parse::<DerivationPath>() {
                Ok(v) => assert_eq!(v.0, case.1),
                Err(e) => panic!("unexpected error {}", e),
            }
        }
    }

    #[test]
    fn it_handles_malformatted_derivations() {
        let cases = ["//", "m/", "-", "h", "toast", "憂鬱"];
        for case in cases.iter() {
            match case.parse::<DerivationPath>() {
                Ok(_) => panic!("expected an error"),
                Err(Bip32Error::MalformattedDerivation(e)) => assert_eq!(&e, case),
                Err(e) => panic!("unexpected error {}", e),
            }
        }
    }

    #[test]
    fn it_removes_prefixes_from_derivations() {
        // express each row in a separate instantiation syntax :)
        let cases = [
            (
                DerivationPath(vec![1, 2, 3]),
                DerivationPath(vec![1]),
                Some(DerivationPath(vec![2, 3])),
            ),
            (
                vec![1, 2, 3].into(),
                vec![1, 2].into(),
                Some(vec![3].into()),
            ),
            (
                (1u32..=3).collect(),
                (1u32..=3).collect(),
                Some((0..0).collect()),
            ),
            (DerivationPath(vec![1, 2, 3]), vec![1, 3].into(), None),
        ];
        for case in cases.iter() {
            assert_eq!(case.0.without_prefix(&case.1), case.2);
        }
    }

    #[test]
    fn it_proudces_paths_from_strings() {
        let cases = ["//", "m/", "-", "h", "toast", "憂鬱"];

        for case in cases.iter() {
            let path: Result<DerivationPath, _> = case.parse().map_err(Into::into);
            match path {
                Ok(_) => panic!("expected an error"),
                Err(Bip32Error::MalformattedDerivation(e)) => assert_eq!(&e, case),
                Err(e) => panic!("unexpected error {}", e),
            }
        }
    }

    #[test]
    fn it_stringifies_derivation_paths() {
        let cases = [
            (DerivationPath(vec![1, 2, 3]), "m/1/2/3"),
            (
                vec![BIP32_HARDEN, BIP32_HARDEN, BIP32_HARDEN].into(),
                "m/0'/0'/0'",
            ),
        ];
        for case in cases.iter() {
            assert_eq!(&case.0.derivation_string(), case.1);
        }
    }
}
