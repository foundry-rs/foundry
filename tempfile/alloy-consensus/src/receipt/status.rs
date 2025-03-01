use alloy_primitives::B256;
use alloy_rlp::{Buf, BufMut, Decodable, Encodable, Error, Header};

/// Captures the result of a transaction execution.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
pub enum Eip658Value {
    /// A boolean `statusCode` introduced by [EIP-658].
    ///
    /// [EIP-658]: https://eips.ethereum.org/EIPS/eip-658
    Eip658(bool),
    /// A pre-[EIP-658] hash value.
    ///
    /// [EIP-658]: https://eips.ethereum.org/EIPS/eip-658
    PostState(B256),
}

impl Eip658Value {
    /// Returns a successful transaction status.
    pub const fn success() -> Self {
        Self::Eip658(true)
    }

    /// Returns true if the transaction was successful OR if the transaction
    /// is pre-[EIP-658].
    ///
    /// [EIP-658]: https://eips.ethereum.org/EIPS/eip-658
    pub const fn coerce_status(&self) -> bool {
        matches!(self, Self::Eip658(true) | Self::PostState(_))
    }

    /// Returns true if the transaction was a pre-[EIP-658] transaction.
    ///
    /// [EIP-658]: https://eips.ethereum.org/EIPS/eip-658
    pub const fn is_post_state(&self) -> bool {
        matches!(self, Self::PostState(_))
    }

    /// Returns true if the transaction was a post-[EIP-658] transaction.
    pub const fn is_eip658(&self) -> bool {
        !matches!(self, Self::PostState(_))
    }

    /// Fallibly convert to the post state.
    pub const fn as_post_state(&self) -> Option<B256> {
        match self {
            Self::PostState(state) => Some(*state),
            _ => None,
        }
    }

    /// Fallibly convert to the [EIP-658] status code.
    ///
    /// [EIP-658]: https://eips.ethereum.org/EIPS/eip-658
    pub const fn as_eip658(&self) -> Option<bool> {
        match self {
            Self::Eip658(status) => Some(*status),
            _ => None,
        }
    }
}

impl From<bool> for Eip658Value {
    fn from(status: bool) -> Self {
        Self::Eip658(status)
    }
}

impl From<B256> for Eip658Value {
    fn from(state: B256) -> Self {
        Self::PostState(state)
    }
}

// NB: default to success
impl Default for Eip658Value {
    fn default() -> Self {
        Self::Eip658(true)
    }
}

#[cfg(feature = "serde")]
mod serde_eip658 {
    //! Serde implementation for [`Eip658Value`]. Serializes [`Eip658Value::Eip658`] as `status`
    //! key, and [`Eip658Value::PostState`] as `root` key.
    //!
    //! If both are present, prefers `status` key.
    //!
    //! Should be used with `#[serde(flatten)]`.
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(untagged)]
    enum SerdeHelper {
        Eip658 {
            #[serde(with = "alloy_serde::quantity")]
            status: bool,
        },
        PostState {
            root: B256,
        },
    }

    impl Serialize for Eip658Value {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            match self {
                Self::Eip658(status) => {
                    SerdeHelper::Eip658 { status: *status }.serialize(serializer)
                }
                Self::PostState(state) => {
                    SerdeHelper::PostState { root: *state }.serialize(serializer)
                }
            }
        }
    }

    impl<'de> Deserialize<'de> for Eip658Value {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let helper = SerdeHelper::deserialize(deserializer)?;
            match helper {
                SerdeHelper::Eip658 { status } => Ok(Self::Eip658(status)),
                SerdeHelper::PostState { root } => Ok(Self::PostState(root)),
            }
        }
    }
}

impl Encodable for Eip658Value {
    fn encode(&self, buf: &mut dyn BufMut) {
        match self {
            Self::Eip658(status) => {
                status.encode(buf);
            }
            Self::PostState(state) => {
                state.encode(buf);
            }
        }
    }

    fn length(&self) -> usize {
        match self {
            Self::Eip658(inner) => inner.length(),
            Self::PostState(inner) => inner.length(),
        }
    }
}

impl Decodable for Eip658Value {
    fn decode(buf: &mut &[u8]) -> Result<Self, Error> {
        let h = Header::decode(buf)?;

        match h.payload_length {
            0 => Ok(Self::Eip658(false)),
            1 => {
                let status = buf.get_u8() != 0;
                Ok(status.into())
            }
            32 => {
                if buf.remaining() < 32 {
                    return Err(Error::InputTooShort);
                }
                let mut state = B256::default();
                buf.copy_to_slice(state.as_mut_slice());
                Ok(state.into())
            }
            _ => Err(Error::UnexpectedLength),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn rlp_sanity() {
        let mut buf = Vec::new();
        let status = Eip658Value::Eip658(true);
        status.encode(&mut buf);
        assert_eq!(Eip658Value::decode(&mut buf.as_slice()), Ok(status));

        let mut buf = Vec::new();
        let state = Eip658Value::PostState(B256::default());
        state.encode(&mut buf);
        assert_eq!(Eip658Value::decode(&mut buf.as_slice()), Ok(state));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_sanity() {
        let status: Eip658Value = true.into();
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#"{"status":"0x1"}"#);
        assert_eq!(serde_json::from_str::<Eip658Value>(&json).unwrap(), status);

        let state: Eip658Value = false.into();
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, r#"{"status":"0x0"}"#);

        let state: Eip658Value = B256::repeat_byte(1).into();
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(
            json,
            r#"{"root":"0x0101010101010101010101010101010101010101010101010101010101010101"}"#
        );
    }
}
