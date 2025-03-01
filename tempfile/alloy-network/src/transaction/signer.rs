use crate::{Network, TransactionBuilder};
use alloy_consensus::SignableTransaction;
use alloy_primitives::Address;
use alloy_signer::{Signer, SignerSync};
use async_trait::async_trait;
use auto_impl::auto_impl;
use futures_utils_wasm::impl_future;

/// A wallet capable of signing any transaction for the given network.
///
/// Network crate authors should implement this trait on a type capable of
/// signing any transaction (regardless of signature type) on a given network.
/// Signer crate authors should instead implement [`TxSigner`] to signify
/// signing capability for specific signature types.
///
/// Network wallets are expected to contain one or more signing credentials,
/// keyed by signing address. The default signer address should be used when
/// no specific signer address is specified.
#[auto_impl(&, &mut, Box, Rc, Arc)]
pub trait NetworkWallet<N: Network>: std::fmt::Debug + Send + Sync {
    /// Get the default signer address. This address should be used
    /// in [`NetworkWallet::sign_transaction_from`] when no specific signer is
    /// specified.
    fn default_signer_address(&self) -> Address;

    /// Return true if the signer contains a credential for the given address.
    fn has_signer_for(&self, address: &Address) -> bool;

    /// Return an iterator of all signer addresses.
    fn signer_addresses(&self) -> impl Iterator<Item = Address>;

    /// Asynchronously sign an unsigned transaction, with a specified
    /// credential.
    #[doc(alias = "sign_tx_from")]
    fn sign_transaction_from(
        &self,
        sender: Address,
        tx: N::UnsignedTx,
    ) -> impl_future!(<Output = alloy_signer::Result<N::TxEnvelope>>);

    /// Asynchronously sign an unsigned transaction.
    #[doc(alias = "sign_tx")]
    fn sign_transaction(
        &self,
        tx: N::UnsignedTx,
    ) -> impl_future!(<Output = alloy_signer::Result<N::TxEnvelope>>) {
        self.sign_transaction_from(self.default_signer_address(), tx)
    }

    /// Asynchronously sign a transaction request, using the sender specified
    /// in the `from` field.
    fn sign_request(
        &self,
        request: N::TransactionRequest,
    ) -> impl_future!(<Output = alloy_signer::Result<N::TxEnvelope>>) {
        async move {
            let sender = request.from().unwrap_or_else(|| self.default_signer_address());
            let tx = request.build_unsigned().map_err(alloy_signer::Error::other)?;
            self.sign_transaction_from(sender, tx).await
        }
    }
}

/// Asynchronous transaction signer, capable of signing any [`SignableTransaction`] for the given
/// `Signature` type.
///
/// A signer should hold an optional [`ChainId`] value, which is used for [EIP-155] replay
/// protection.
///
/// If `chain_id` is Some, [EIP-155] should be applied to the input transaction in
/// [`sign_transaction`](Self::sign_transaction), and to the resulting signature in all the methods.
/// If `chain_id` is None, [EIP-155] should not be applied.
///
/// Synchronous signers should implement both this trait and [`TxSignerSync`].
///
/// [EIP-155]: https://eips.ethereum.org/EIPS/eip-155
/// [`ChainId`]: alloy_primitives::ChainId
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[auto_impl(&, &mut, Box, Rc, Arc)]
#[doc(alias = "TransactionSigner")]
pub trait TxSigner<Signature> {
    /// Get the address of the signer.
    fn address(&self) -> Address;

    /// Asynchronously sign an unsigned transaction.
    #[doc(alias = "sign_tx")]
    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> alloy_signer::Result<Signature>;
}

/// Synchronous transaction signer,  capable of signing any [`SignableTransaction`] for the given
/// `Signature` type.
///
/// A signer should hold an optional [`ChainId`] value, which is used for [EIP-155] replay
/// protection.
///
/// If `chain_id` is Some, [EIP-155] should be applied to the input transaction in
/// [`sign_transaction_sync`](Self::sign_transaction_sync), and to the resulting signature in all
/// the methods. If `chain_id` is None, [EIP-155] should not be applied.
///
/// Synchronous signers should also implement [`TxSigner`], as they are always able to by delegating
/// the asynchronous methods to the synchronous ones.
///
/// [EIP-155]: https://eips.ethereum.org/EIPS/eip-155
/// [`ChainId`]: alloy_primitives::ChainId
#[auto_impl(&, &mut, Box, Rc, Arc)]
#[doc(alias = "TransactionSignerSync")]
pub trait TxSignerSync<Signature> {
    /// Get the address of the signer.
    fn address(&self) -> Address;

    /// Synchronously sign an unsigned transaction.
    #[doc(alias = "sign_tx_sync")]
    fn sign_transaction_sync(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> alloy_signer::Result<Signature>;
}

/// A unifying trait for asynchronous Ethereum signers that combine the functionalities of both
/// [`Signer`] and [`TxSigner`].
///
/// This trait enables dynamic dispatch (e.g., using `Box<dyn FullSigner>`) for types that combine
/// both asynchronous Ethereum signing and transaction signing functionalities.
pub trait FullSigner<S>: Signer<S> + TxSigner<S> {}
impl<T, S> FullSigner<S> for T where T: Signer<S> + TxSigner<S> {}

/// A unifying trait for synchronous Ethereum signers that implement both [`SignerSync`] and
/// [`TxSignerSync`].
///
/// This trait enables dynamic dispatch (e.g., using `Box<dyn FullSignerSync>`) for types that
/// combine both synchronous Ethereum signing and transaction signing functionalities.
pub trait FullSignerSync<S>: SignerSync<S> + TxSignerSync<S> {}
impl<T, S> FullSignerSync<S> for T where T: SignerSync<S> + TxSignerSync<S> {}

#[cfg(test)]
mod tests {
    use super::*;

    struct _ObjectSafe(Box<dyn FullSigner<()>>, Box<dyn FullSignerSync<()>>);
}
