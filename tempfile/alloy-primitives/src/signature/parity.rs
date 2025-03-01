use crate::{
    signature::{utils::normalize_v_to_byte, SignatureError},
    to_eip155_v, ChainId, Uint, U64,
};

/// The parity of the signature, stored as either a V value (which may include
/// a chain id), or the y-parity.
#[deprecated(since = "0.8.15", note = "see https://github.com/alloy-rs/core/pull/776")]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(derive_arbitrary::Arbitrary, proptest_derive::Arbitrary))]
pub enum Parity {
    /// Explicit V value. May be EIP-155 modified.
    Eip155(u64),
    /// Non-EIP155. 27 or 28.
    NonEip155(bool),
    /// Parity flag. True for odd.
    Parity(bool),
}

impl Default for Parity {
    fn default() -> Self {
        Self::Parity(false)
    }
}

#[cfg(feature = "k256")]
impl From<k256::ecdsa::RecoveryId> for Parity {
    fn from(value: k256::ecdsa::RecoveryId) -> Self {
        Self::Parity(value.is_y_odd())
    }
}

impl TryFrom<U64> for Parity {
    type Error = <Self as TryFrom<u64>>::Error;
    fn try_from(value: U64) -> Result<Self, Self::Error> {
        value.as_limbs()[0].try_into()
    }
}

impl From<Uint<1, 1>> for Parity {
    fn from(value: Uint<1, 1>) -> Self {
        Self::Parity(!value.is_zero())
    }
}

impl From<bool> for Parity {
    fn from(value: bool) -> Self {
        Self::Parity(value)
    }
}

impl TryFrom<u64> for Parity {
    type Error = SignatureError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 | 1 => Ok(Self::Parity(value != 0)),
            27 | 28 => Ok(Self::NonEip155((value - 27) != 0)),
            value @ 35..=u64::MAX => Ok(Self::Eip155(value)),
            _ => Err(SignatureError::InvalidParity(value)),
        }
    }
}

impl Parity {
    /// Returns the chain ID associated with the V value, if this signature is
    /// replay-protected by [EIP-155].
    ///
    /// [EIP-155]: https://eips.ethereum.org/EIPS/eip-155
    pub const fn chain_id(&self) -> Option<ChainId> {
        match *self {
            Self::Eip155(mut v @ 35..) => {
                if v % 2 == 0 {
                    v -= 1;
                }
                v -= 35;
                Some(v / 2)
            }
            _ => None,
        }
    }

    /// Returns true if the signature is replay-protected by [EIP-155].
    ///
    /// This is true if the V value is 35 or greater. Values less than 35 are
    /// either not replay protected (27/28), or are invalid.
    ///
    /// [EIP-155]: https://eips.ethereum.org/EIPS/eip-155
    pub const fn has_eip155_value(&self) -> bool {
        self.chain_id().is_some()
    }

    /// Return the y-parity as a boolean.
    pub const fn y_parity(&self) -> bool {
        match self {
            Self::Eip155(v @ 0..=34) => *v % 2 == 1,
            Self::Eip155(v) => (*v ^ 1) % 2 == 1,
            Self::NonEip155(b) | Self::Parity(b) => *b,
        }
    }

    /// Return the y-parity as 0 or 1
    pub const fn y_parity_byte(&self) -> u8 {
        self.y_parity() as u8
    }

    /// Return the y-parity byte as 27 or 28,
    /// in the case of a non-EIP155 signature.
    pub const fn y_parity_byte_non_eip155(&self) -> Option<u8> {
        match self {
            Self::NonEip155(v) | Self::Parity(v) => Some(*v as u8 + 27),
            _ => None,
        }
    }

    /// Return the corresponding u64 V value.
    pub const fn to_u64(&self) -> u64 {
        match self {
            Self::Eip155(v) => *v,
            Self::NonEip155(b) => *b as u64 + 27,
            Self::Parity(b) => *b as u64,
        }
    }

    /// Inverts the parity.
    pub const fn inverted(&self) -> Self {
        match *self {
            Self::Parity(b) => Self::Parity(!b),
            Self::NonEip155(b) => Self::NonEip155(!b),
            Self::Eip155(0) => Self::Eip155(1),
            Self::Eip155(v @ 1..=34) => Self::Eip155(if v % 2 == 0 { v - 1 } else { v + 1 }),
            Self::Eip155(v @ 35..) => Self::Eip155(v ^ 1),
        }
    }

    /// Converts an EIP-155 V value to a non-EIP-155 V value.
    ///
    /// This is a nop for non-EIP-155 values.
    pub const fn strip_chain_id(&self) -> Self {
        match *self {
            Self::Eip155(v) => Self::NonEip155(v % 2 == 1),
            this => this,
        }
    }

    /// Applies EIP-155 with the given chain ID.
    pub const fn with_chain_id(self, chain_id: ChainId) -> Self {
        let parity = match self {
            Self::Eip155(v) => normalize_v_to_byte(v) == 1,
            Self::NonEip155(b) | Self::Parity(b) => b,
        };

        Self::Eip155(to_eip155_v(parity as u8, chain_id))
    }

    /// Determines the recovery ID.
    #[cfg(feature = "k256")]
    pub const fn recid(&self) -> k256::ecdsa::RecoveryId {
        let recid_opt = match self {
            Self::Eip155(v) => Some(crate::signature::utils::normalize_v_to_recid(*v)),
            Self::NonEip155(b) | Self::Parity(b) => k256::ecdsa::RecoveryId::from_byte(*b as u8),
        };

        // manual unwrap for const fn
        match recid_opt {
            Some(recid) => recid,
            None => unreachable!(),
        }
    }

    /// Convert to a parity bool, dropping any V information.
    pub const fn to_parity_bool(self) -> Self {
        Self::Parity(self.y_parity())
    }
}

#[cfg(feature = "rlp")]
impl alloy_rlp::Encodable for Parity {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        match self {
            Self::Eip155(v) => v.encode(out),
            Self::NonEip155(v) => (*v as u8 + 27).encode(out),
            Self::Parity(b) => b.encode(out),
        }
    }

    fn length(&self) -> usize {
        match self {
            Self::Eip155(v) => v.length(),
            Self::NonEip155(_) => 0u8.length(),
            Self::Parity(v) => v.length(),
        }
    }
}

#[cfg(feature = "rlp")]
impl alloy_rlp::Decodable for Parity {
    fn decode(buf: &mut &[u8]) -> Result<Self, alloy_rlp::Error> {
        let v = u64::decode(buf)?;
        Ok(match v {
            0 => Self::Parity(false),
            1 => Self::Parity(true),
            27 => Self::NonEip155(false),
            28 => Self::NonEip155(true),
            v @ 35..=u64::MAX => Self::try_from(v).expect("checked range"),
            _ => return Err(alloy_rlp::Error::Custom("Invalid parity value")),
        })
    }
}

#[cfg(test)]
mod test {
    use crate::Parity;

    #[cfg(feature = "rlp")]
    #[test]
    fn basic_rlp() {
        use crate::hex;
        use alloy_rlp::{Decodable, Encodable};

        let vector = vec![
            (hex!("01").as_slice(), Parity::Parity(true)),
            (hex!("1b").as_slice(), Parity::NonEip155(false)),
            (hex!("25").as_slice(), Parity::Eip155(37)),
            (hex!("26").as_slice(), Parity::Eip155(38)),
            (hex!("81ff").as_slice(), Parity::Eip155(255)),
        ];

        for test in vector.into_iter() {
            let mut buf = vec![];
            test.1.encode(&mut buf);
            assert_eq!(test.0, buf.as_slice());

            assert_eq!(test.1, Parity::decode(&mut buf.as_slice()).unwrap());
        }
    }

    #[test]
    fn u64_round_trip() {
        let parity = Parity::Eip155(37);
        assert_eq!(parity, Parity::try_from(parity.to_u64()).unwrap());
        let parity = Parity::Eip155(38);
        assert_eq!(parity, Parity::try_from(parity.to_u64()).unwrap());
        let parity = Parity::NonEip155(false);
        assert_eq!(parity, Parity::try_from(parity.to_u64()).unwrap());
        let parity = Parity::NonEip155(true);
        assert_eq!(parity, Parity::try_from(parity.to_u64()).unwrap());
        let parity = Parity::Parity(false);
        assert_eq!(parity, Parity::try_from(parity.to_u64()).unwrap());
        let parity = Parity::Parity(true);
        assert_eq!(parity, Parity::try_from(parity.to_u64()).unwrap());
    }

    #[test]
    fn round_trip() {
        // with chain ID 1
        let p = Parity::Eip155(37);

        assert_eq!(p.to_parity_bool(), Parity::Parity(false));

        assert_eq!(p.with_chain_id(1), Parity::Eip155(37));
    }

    #[test]
    fn invert_parity() {
        let p = Parity::Eip155(0);
        assert_eq!(p.inverted(), Parity::Eip155(1));

        let p = Parity::Eip155(22);
        assert_eq!(p.inverted(), Parity::Eip155(21));

        let p = Parity::Eip155(58);
        assert_eq!(p.inverted(), Parity::Eip155(59));

        let p = Parity::NonEip155(false);
        assert_eq!(p.inverted(), Parity::NonEip155(true));

        let p = Parity::Parity(true);
        assert_eq!(p.inverted(), Parity::Parity(false));
    }
}
