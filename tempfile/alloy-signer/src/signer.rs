use crate::Result;
use alloy_primitives::{
    eip191_hash_message, Address, ChainId, PrimitiveSignature as Signature, B256,
};
use async_trait::async_trait;
use auto_impl::auto_impl;
pub use either::Either;

#[cfg(feature = "eip712")]
use alloy_dyn_abi::eip712::TypedData;
#[cfg(feature = "eip712")]
use alloy_sol_types::{Eip712Domain, SolStruct};

/// Asynchronous Ethereum signer.
///
/// All provided implementations rely on [`sign_hash`](Signer::sign_hash). A signer may not always
/// be able to implement this method, in which case it should return
/// [`UnsupportedOperation`](crate::Error::UnsupportedOperation), and implement all the signing
/// methods directly.
///
/// Synchronous signers should implement both this trait and [`SignerSync`].
///
/// [EIP-155]: https://eips.ethereum.org/EIPS/eip-155
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[auto_impl(&mut, Box)]
pub trait Signer<Sig = Signature> {
    /// Signs the given hash.
    async fn sign_hash(&self, hash: &B256) -> Result<Sig>;

    /// Signs the hash of the provided message after prefixing it, as specified in [EIP-191].
    ///
    /// [EIP-191]: https://eips.ethereum.org/EIPS/eip-191
    #[inline]
    async fn sign_message(&self, message: &[u8]) -> Result<Sig> {
        self.sign_hash(&eip191_hash_message(message)).await
    }

    /// Encodes and signs the typed data according to [EIP-712].
    ///
    /// [EIP-712]: https://eips.ethereum.org/EIPS/eip-712
    #[cfg(feature = "eip712")]
    #[inline]
    #[auto_impl(keep_default_for(&mut, Box))]
    async fn sign_typed_data<T: SolStruct + Send + Sync>(
        &self,
        payload: &T,
        domain: &Eip712Domain,
    ) -> Result<Sig>
    where
        Self: Sized,
    {
        self.sign_hash(&payload.eip712_signing_hash(domain)).await
    }

    /// Encodes and signs the typed data according to [EIP-712] for Signers that are not dynamically
    /// sized.
    #[cfg(feature = "eip712")]
    #[inline]
    async fn sign_dynamic_typed_data(&self, payload: &TypedData) -> Result<Sig> {
        self.sign_hash(&payload.eip712_signing_hash()?).await
    }

    /// Returns the signer's Ethereum Address.
    fn address(&self) -> Address;

    /// Returns the signer's chain ID.
    fn chain_id(&self) -> Option<ChainId>;

    /// Sets the signer's chain ID.
    fn set_chain_id(&mut self, chain_id: Option<ChainId>);

    /// Sets the signer's chain ID and returns `self`.
    #[inline]
    #[must_use]
    #[auto_impl(keep_default_for(&mut, Box))]
    fn with_chain_id(mut self, chain_id: Option<ChainId>) -> Self
    where
        Self: Sized,
    {
        self.set_chain_id(chain_id);
        self
    }
}

/// Synchronous Ethereum signer.
///
/// All provided implementations rely on [`sign_hash_sync`](SignerSync::sign_hash_sync). A signer
/// may not always be able to implement this method, in which case it should return
/// [`UnsupportedOperation`](crate::Error::UnsupportedOperation), and implement all the signing
/// methods directly.
///
/// Synchronous signers should also implement [`Signer`], as they are always able to by delegating
/// the asynchronous methods to the synchronous ones.
///
/// [EIP-155]: https://eips.ethereum.org/EIPS/eip-155
#[auto_impl(&, &mut, Box, Rc, Arc)]
pub trait SignerSync<Sig = Signature> {
    /// Signs the given hash.
    fn sign_hash_sync(&self, hash: &B256) -> Result<Sig>;

    /// Signs the hash of the provided message after prefixing it, as specified in [EIP-191].
    ///
    /// [EIP-191]: https://eips.ethereum.org/EIPS/eip-191
    #[inline]
    fn sign_message_sync(&self, message: &[u8]) -> Result<Sig> {
        self.sign_hash_sync(&eip191_hash_message(message))
    }

    /// Encodes and signs the typed data according to [EIP-712].
    ///
    /// [EIP-712]: https://eips.ethereum.org/EIPS/eip-712
    #[cfg(feature = "eip712")]
    #[inline]
    #[auto_impl(keep_default_for(&, &mut, Box, Rc, Arc))]
    fn sign_typed_data_sync<T: SolStruct>(&self, payload: &T, domain: &Eip712Domain) -> Result<Sig>
    where
        Self: Sized,
    {
        self.sign_hash_sync(&payload.eip712_signing_hash(domain))
    }

    /// Encodes and signs the typed data according to [EIP-712] for Signers that are not dynamically
    /// sized.
    ///
    /// [EIP-712]: https://eips.ethereum.org/EIPS/eip-712
    #[cfg(feature = "eip712")]
    #[inline]
    fn sign_dynamic_typed_data_sync(&self, payload: &TypedData) -> Result<Sig> {
        let hash = payload.eip712_signing_hash()?;
        self.sign_hash_sync(&hash)
    }

    /// Returns the signer's chain ID.
    fn chain_id_sync(&self) -> Option<ChainId>;
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[async_trait]
impl<A, B, Sig> Signer<Sig> for Either<A, B>
where
    A: Signer<Sig> + Send + Sync,
    B: Signer<Sig> + Send + Sync,
    Sig: Send,
{
    async fn sign_hash(&self, hash: &B256) -> Result<Sig> {
        match self {
            Self::Left(signer) => signer.sign_hash(hash).await,
            Self::Right(signer) => signer.sign_hash(hash).await,
        }
    }

    fn address(&self) -> Address {
        match self {
            Self::Left(signer) => signer.address(),
            Self::Right(signer) => signer.address(),
        }
    }

    fn chain_id(&self) -> Option<ChainId> {
        match self {
            Self::Left(signer) => signer.chain_id(),
            Self::Right(signer) => signer.chain_id(),
        }
    }

    fn set_chain_id(&mut self, chain_id: Option<ChainId>) {
        match self {
            Self::Left(signer) => signer.set_chain_id(chain_id),
            Self::Right(signer) => signer.set_chain_id(chain_id),
        }
    }
}

impl<A, B, Sig> SignerSync<Sig> for Either<A, B>
where
    A: SignerSync<Sig>,
    B: SignerSync<Sig>,
{
    fn sign_hash_sync(&self, hash: &B256) -> Result<Sig> {
        match self {
            Self::Left(signer) => signer.sign_hash_sync(hash),
            Self::Right(signer) => signer.sign_hash_sync(hash),
        }
    }

    fn chain_id_sync(&self) -> Option<ChainId> {
        match self {
            Self::Left(signer) => signer.chain_id_sync(),
            Self::Right(signer) => signer.chain_id_sync(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, UnsupportedSignerOperation};
    use assert_matches::assert_matches;
    use std::sync::Arc;

    struct _ObjectSafe(Box<dyn Signer>, Box<dyn SignerSync>);

    #[tokio::test]
    async fn unimplemented() {
        #[cfg(feature = "eip712")]
        alloy_sol_types::sol! {
            #[derive(Default, serde::Serialize)]
            struct Eip712Data {
                uint64 a;
            }
        }

        async fn test_unimplemented_signer<S: Signer + SignerSync + Send + Sync>(s: &S) {
            test_unsized_unimplemented_signer(s).await;
            test_unsized_unimplemented_signer_sync(s);

            #[cfg(feature = "eip712")]
            assert!(s
                .sign_typed_data_sync(&Eip712Data::default(), &Eip712Domain::default())
                .is_err());
            #[cfg(feature = "eip712")]
            assert!(s
                .sign_typed_data(&Eip712Data::default(), &Eip712Domain::default())
                .await
                .is_err());
        }

        async fn test_unsized_unimplemented_signer<S: Signer + ?Sized + Send + Sync>(s: &S) {
            assert_matches!(
                s.sign_hash(&B256::ZERO).await,
                Err(Error::UnsupportedOperation(UnsupportedSignerOperation::SignHash))
            );

            assert_matches!(
                s.sign_message(&[]).await,
                Err(Error::UnsupportedOperation(UnsupportedSignerOperation::SignHash))
            );

            #[cfg(feature = "eip712")]
            assert_matches!(
                s.sign_dynamic_typed_data(&TypedData::from_struct(&Eip712Data::default(), None))
                    .await,
                Err(Error::UnsupportedOperation(UnsupportedSignerOperation::SignHash))
            );

            assert_eq!(s.chain_id(), None);
        }

        fn test_unsized_unimplemented_signer_sync<S: SignerSync + ?Sized>(s: &S) {
            assert_matches!(
                s.sign_hash_sync(&B256::ZERO),
                Err(Error::UnsupportedOperation(UnsupportedSignerOperation::SignHash))
            );

            assert_matches!(
                s.sign_message_sync(&[]),
                Err(Error::UnsupportedOperation(UnsupportedSignerOperation::SignHash))
            );

            #[cfg(feature = "eip712")]
            assert_matches!(
                s.sign_dynamic_typed_data_sync(&TypedData::from_struct(
                    &Eip712Data::default(),
                    None
                )),
                Err(Error::UnsupportedOperation(UnsupportedSignerOperation::SignHash))
            );

            assert_eq!(s.chain_id_sync(), None);
        }

        struct UnimplementedSigner;

        #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
        #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
        impl Signer for UnimplementedSigner {
            async fn sign_hash(&self, _hash: &B256) -> Result<Signature> {
                Err(Error::UnsupportedOperation(UnsupportedSignerOperation::SignHash))
            }

            fn address(&self) -> Address {
                Address::ZERO
            }

            fn chain_id(&self) -> Option<ChainId> {
                None
            }

            fn set_chain_id(&mut self, _chain_id: Option<ChainId>) {}
        }

        impl SignerSync for UnimplementedSigner {
            fn sign_hash_sync(&self, _hash: &B256) -> Result<Signature> {
                Err(Error::UnsupportedOperation(UnsupportedSignerOperation::SignHash))
            }

            fn chain_id_sync(&self) -> Option<ChainId> {
                None
            }
        }

        test_unimplemented_signer(&UnimplementedSigner).await;
        test_unsized_unimplemented_signer(&UnimplementedSigner as &(dyn Signer + Send + Sync))
            .await;
        test_unsized_unimplemented_signer_sync(
            &UnimplementedSigner as &(dyn SignerSync + Send + Sync),
        );

        test_unsized_unimplemented_signer(
            &(Box::new(UnimplementedSigner) as Box<dyn Signer + Send + Sync>),
        )
        .await;
        test_unsized_unimplemented_signer_sync(
            &(Box::new(UnimplementedSigner) as Box<dyn SignerSync + Send + Sync>),
        );

        test_unsized_unimplemented_signer_sync(
            &(Arc::new(UnimplementedSigner) as Arc<dyn SignerSync + Send + Sync>),
        );
    }
}
