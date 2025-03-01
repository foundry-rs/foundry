use alloy_eips::{eip2718::Encodable2718, Typed2718};
use alloy_primitives::{bytes, Address, B256};
use alloy_rlp::{Decodable, Encodable};
use derive_more::{AsRef, Deref};

/// Signed transaction with recovered signer.
#[derive(Debug, Clone, PartialEq, Hash, Eq, AsRef, Deref)]
pub struct Recovered<T> {
    /// Signer of the transaction
    signer: Address,
    /// Signed transaction
    #[deref]
    #[as_ref]
    tx: T,
}

impl<T> Recovered<T> {
    /// Signer of transaction recovered from signature
    pub const fn signer(&self) -> Address {
        self.signer
    }

    /// Reference to the signer of transaction recovered from signature
    pub const fn signer_ref(&self) -> &Address {
        &self.signer
    }

    /// Returns a reference to the transaction.
    #[doc(alias = "transaction")]
    pub const fn tx(&self) -> &T {
        &self.tx
    }

    /// Transform back to the transaction.
    #[doc(alias = "into_transaction")]
    pub fn into_tx(self) -> T {
        self.tx
    }

    /// Clone the inner transaction.
    #[doc(alias = "clone_transaction")]
    pub fn clone_tx(&self) -> T
    where
        T: Clone,
    {
        self.tx.clone()
    }

    /// Dissolve Self to its component
    #[doc(alias = "split")]
    pub fn into_parts(self) -> (T, Address) {
        (self.tx, self.signer)
    }

    /// Converts from `&Recovered<T>` to `Recovered<&T>`.
    pub const fn as_recovered_ref(&self) -> Recovered<&T> {
        Recovered { tx: &self.tx, signer: self.signer() }
    }

    /// Create [`Recovered`] from the given transaction and [`Address`] of the signer.
    ///
    /// Note: This does not check if the signer is the actual signer of the transaction.
    #[inline]
    pub const fn new_unchecked(tx: T, signer: Address) -> Self {
        Self { tx, signer }
    }

    /// Applies the given closure to the inner transaction type.
    pub fn map_transaction<Tx>(self, f: impl FnOnce(T) -> Tx) -> Recovered<Tx> {
        Recovered::new_unchecked(f(self.tx), self.signer)
    }

    /// Applies the given fallible closure to the inner transaction type.
    pub fn try_map_transaction<Tx, E>(
        self,
        f: impl FnOnce(T) -> Result<Tx, E>,
    ) -> Result<Recovered<Tx>, E> {
        Ok(Recovered::new_unchecked(f(self.tx)?, self.signer))
    }
}

impl<T> Recovered<&T> {
    /// Maps a `Recovered<&T>` to a `Recovered<T>` by cloning the transaction.
    pub fn cloned(self) -> Recovered<T>
    where
        T: Clone,
    {
        let Self { tx, signer } = self;
        Recovered::new_unchecked(tx.clone(), signer)
    }
}

impl<T: Encodable> Encodable for Recovered<T> {
    /// This encodes the transaction _with_ the signature, and an rlp header.
    fn encode(&self, out: &mut dyn bytes::BufMut) {
        self.tx.encode(out)
    }

    fn length(&self) -> usize {
        self.tx.length()
    }
}

impl<T: Decodable + SignerRecoverable> Decodable for Recovered<T> {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let tx = T::decode(buf)?;
        let signer = tx.recover_signer().map_err(|_| {
            alloy_rlp::Error::Custom("Unable to recover decoded transaction signer.")
        })?;
        Ok(Self::new_unchecked(tx, signer))
    }
}

impl<T: Typed2718> Typed2718 for Recovered<T> {
    fn ty(&self) -> u8 {
        self.tx.ty()
    }
}

impl<T: Encodable2718> Encodable2718 for Recovered<T> {
    fn encode_2718_len(&self) -> usize {
        self.tx.encode_2718_len()
    }

    fn encode_2718(&self, out: &mut dyn alloy_rlp::BufMut) {
        self.tx.encode_2718(out)
    }

    fn trie_hash(&self) -> B256 {
        self.tx.trie_hash()
    }
}

/// A type that can recover the signer of a transaction.
///
/// This is a helper trait that only provides the ability to recover the signer (address) of a
/// transaction.
pub trait SignerRecoverable {
    /// Recover signer from signature and hash.
    ///
    /// Returns an error if the transaction's signature is invalid following [EIP-2](https://eips.ethereum.org/EIPS/eip-2).
    ///
    /// Note:
    ///
    /// This can fail for some early ethereum mainnet transactions pre EIP-2, use
    /// [`Self::recover_signer_unchecked`] if you want to recover the signer without ensuring that
    /// the signature has a low `s` value.
    fn recover_signer(&self) -> Result<Address, alloy_primitives::SignatureError>;

    /// Recover signer from signature and hash _without ensuring that the signature has a low `s`
    /// value_.
    ///
    /// Returns an error if the transaction's signature is invalid.
    fn recover_signer_unchecked(&self) -> Result<Address, alloy_primitives::SignatureError>;
}
