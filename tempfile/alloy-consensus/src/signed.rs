use crate::transaction::{RlpEcdsaTx, SignableTransaction};
use alloy_eips::eip2718::Eip2718Result;
use alloy_primitives::{PrimitiveSignature as Signature, B256};
use alloy_rlp::BufMut;

/// A transaction with a signature and hash seal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Signed<T, Sig = Signature> {
    #[cfg_attr(feature = "serde", serde(flatten))]
    #[doc(alias = "transaction")]
    tx: T,
    #[cfg_attr(feature = "serde", serde(flatten))]
    signature: Sig,
    #[doc(alias = "tx_hash", alias = "transaction_hash")]
    hash: B256,
}

impl<T, Sig> Signed<T, Sig> {
    /// Instantiate from a transaction and signature. Does not verify the signature.
    pub const fn new_unchecked(tx: T, signature: Sig, hash: B256) -> Self {
        Self { tx, signature, hash }
    }

    /// Returns a reference to the transaction.
    #[doc(alias = "transaction")]
    pub const fn tx(&self) -> &T {
        &self.tx
    }

    /// Returns a mutable reference to the transaction.
    pub fn tx_mut(&mut self) -> &mut T {
        &mut self.tx
    }

    /// Returns a reference to the signature.
    pub const fn signature(&self) -> &Sig {
        &self.signature
    }

    /// Returns a reference to the transaction hash.
    #[doc(alias = "tx_hash", alias = "transaction_hash")]
    pub const fn hash(&self) -> &B256 {
        &self.hash
    }

    /// Splits the transaction into parts.
    pub fn into_parts(self) -> (T, Sig, B256) {
        (self.tx, self.signature, self.hash)
    }

    /// Returns the transaction without signature.
    pub fn strip_signature(self) -> T {
        self.tx
    }
}

impl<T: SignableTransaction<Sig>, Sig> Signed<T, Sig> {
    /// Calculate the signing hash for the transaction.
    pub fn signature_hash(&self) -> B256 {
        self.tx.signature_hash()
    }
}

impl<T> Signed<T>
where
    T: RlpEcdsaTx,
{
    /// Get the length of the transaction when RLP encoded.
    pub fn rlp_encoded_length(&self) -> usize {
        self.tx.rlp_encoded_length_with_signature(&self.signature)
    }

    /// RLP encode the signed transaction.
    pub fn rlp_encode(&self, out: &mut dyn BufMut) {
        self.tx.rlp_encode_signed(&self.signature, out);
    }

    /// Get the length of the transaction when EIP-2718 encoded.
    pub fn eip2718_encoded_length(&self) -> usize {
        self.tx.eip2718_encoded_length(&self.signature)
    }

    /// EIP-2718 encode the signed transaction with a specified type flag.
    pub fn eip2718_encode_with_type(&self, ty: u8, out: &mut dyn BufMut) {
        self.tx.eip2718_encode_with_type(&self.signature, ty, out);
    }

    /// EIP-2718 encode the signed transaction.
    pub fn eip2718_encode(&self, out: &mut dyn BufMut) {
        self.tx.eip2718_encode(&self.signature, out);
    }

    /// Get the length of the transaction when network encoded.
    pub fn network_encoded_length(&self) -> usize {
        self.tx.network_encoded_length(&self.signature)
    }

    /// Network encode the signed transaction with a specified type flag.
    pub fn network_encode_with_type(&self, ty: u8, out: &mut dyn BufMut) {
        self.tx.network_encode_with_type(&self.signature, ty, out);
    }

    /// Network encode the signed transaction.
    pub fn network_encode(&self, out: &mut dyn BufMut) {
        self.tx.network_encode(&self.signature, out);
    }

    /// RLP decode the signed transaction.
    pub fn rlp_decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        T::rlp_decode_signed(buf)
    }

    /// EIP-2718 decode the signed transaction with a specified type flag.
    pub fn eip2718_decode_with_type(buf: &mut &[u8], ty: u8) -> Eip2718Result<Self> {
        T::eip2718_decode_with_type(buf, ty)
    }

    /// EIP-2718 decode the signed transaction.
    pub fn eip2718_decode(buf: &mut &[u8]) -> Eip2718Result<Self> {
        T::eip2718_decode(buf)
    }

    /// Network decode the signed transaction with a specified type flag.
    pub fn network_decode_with_type(buf: &mut &[u8], ty: u8) -> Eip2718Result<Self> {
        T::network_decode_with_type(buf, ty)
    }

    /// Network decode the signed transaction.
    pub fn network_decode(buf: &mut &[u8]) -> Eip2718Result<Self> {
        T::network_decode(buf)
    }
}

#[cfg(feature = "k256")]
impl<T: SignableTransaction<Signature>> Signed<T, Signature> {
    /// Recover the signer of the transaction
    pub fn recover_signer(
        &self,
    ) -> Result<alloy_primitives::Address, alloy_primitives::SignatureError> {
        let sighash = self.tx.signature_hash();
        self.signature.recover_address_from_prehash(&sighash)
    }
}

#[cfg(all(any(test, feature = "arbitrary"), feature = "k256"))]
impl<'a, T: SignableTransaction<Signature> + arbitrary::Arbitrary<'a>> arbitrary::Arbitrary<'a>
    for Signed<T, Signature>
{
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        use k256::{
            ecdsa::{signature::hazmat::PrehashSigner, SigningKey},
            NonZeroScalar,
        };
        use rand::{rngs::StdRng, SeedableRng};

        let rng_seed = u.arbitrary::<[u8; 32]>()?;
        let mut rand_gen = StdRng::from_seed(rng_seed);
        let signing_key: SigningKey = NonZeroScalar::random(&mut rand_gen).into();

        let tx = T::arbitrary(u)?;

        let (recoverable_sig, recovery_id) =
            signing_key.sign_prehash(tx.signature_hash().as_ref()).unwrap();
        let signature: Signature = (recoverable_sig, recovery_id).into();

        Ok(tx.into_signed(signature))
    }
}
