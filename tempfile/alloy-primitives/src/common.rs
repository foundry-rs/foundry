use crate::Address;

#[cfg(feature = "rlp")]
use alloy_rlp::{Buf, BufMut, Decodable, Encodable, EMPTY_STRING_CODE};

/// The `to` field of a transaction. Either a target address, or empty for a
/// contract creation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "arbitrary", derive(derive_arbitrary::Arbitrary, proptest_derive::Arbitrary))]
#[doc(alias = "TransactionKind")]
pub enum TxKind {
    /// A transaction that creates a contract.
    #[default]
    Create,
    /// A transaction that calls a contract or transfer.
    Call(Address),
}

impl From<Option<Address>> for TxKind {
    /// Creates a `TxKind::Call` with the `Some` address, `None` otherwise.
    #[inline]
    fn from(value: Option<Address>) -> Self {
        match value {
            None => Self::Create,
            Some(addr) => Self::Call(addr),
        }
    }
}

impl From<Address> for TxKind {
    /// Creates a `TxKind::Call` with the given address.
    #[inline]
    fn from(value: Address) -> Self {
        Self::Call(value)
    }
}

impl From<TxKind> for Option<Address> {
    /// Returns the address of the contract that will be called or will receive the transfer.
    #[inline]
    fn from(value: TxKind) -> Self {
        value.to().copied()
    }
}

impl TxKind {
    /// Returns the address of the contract that will be called or will receive the transfer.
    pub const fn to(&self) -> Option<&Address> {
        match self {
            Self::Create => None,
            Self::Call(to) => Some(to),
        }
    }

    /// Returns true if the transaction is a contract creation.
    #[inline]
    pub const fn is_create(&self) -> bool {
        matches!(self, Self::Create)
    }

    /// Returns true if the transaction is a contract call.
    #[inline]
    pub const fn is_call(&self) -> bool {
        matches!(self, Self::Call(_))
    }

    /// Calculates a heuristic for the in-memory size of this object.
    #[inline]
    pub const fn size(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

#[cfg(feature = "rlp")]
impl Encodable for TxKind {
    fn encode(&self, out: &mut dyn BufMut) {
        match self {
            Self::Call(to) => to.encode(out),
            Self::Create => out.put_u8(EMPTY_STRING_CODE),
        }
    }

    fn length(&self) -> usize {
        match self {
            Self::Call(to) => to.length(),
            Self::Create => 1, // EMPTY_STRING_CODE is a single byte
        }
    }
}

#[cfg(feature = "rlp")]
impl Decodable for TxKind {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        if let Some(&first) = buf.first() {
            if first == EMPTY_STRING_CODE {
                buf.advance(1);
                Ok(Self::Create)
            } else {
                let addr = <Address as Decodable>::decode(buf)?;
                Ok(Self::Call(addr))
            }
        } else {
            Err(alloy_rlp::Error::InputTooShort)
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for TxKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to().serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for TxKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<Address>::deserialize(deserializer)?.into())
    }
}
