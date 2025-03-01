use crate::{AnyNetwork, AnyTxEnvelope, AnyTypedTransaction, Network, NetworkWallet, TxSigner};
use alloy_consensus::{SignableTransaction, TxEnvelope, TypedTransaction};
use alloy_primitives::{map::AddressHashMap, Address, PrimitiveSignature as Signature};
use std::sync::Arc;

use super::Ethereum;

/// A wallet capable of signing any transaction for the Ethereum network.
#[derive(Clone, Default)]
pub struct EthereumWallet {
    default: Address,
    signers: AddressHashMap<Arc<dyn TxSigner<Signature> + Send + Sync>>,
}

impl std::fmt::Debug for EthereumWallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthereumWallet")
            .field("default_signer", &self.default)
            .field("credentials", &self.signers.len())
            .finish()
    }
}

impl<S> From<S> for EthereumWallet
where
    S: TxSigner<Signature> + Send + Sync + 'static,
{
    fn from(signer: S) -> Self {
        Self::new(signer)
    }
}

impl EthereumWallet {
    /// Create a new signer with the given signer as the default signer.
    pub fn new<S>(signer: S) -> Self
    where
        S: TxSigner<Signature> + Send + Sync + 'static,
    {
        let mut this = Self::default();
        this.register_default_signer(signer);
        this
    }

    /// Register a new signer on this object. This signer will be used to sign
    /// [`TransactionRequest`] and [`TypedTransaction`] object that specify the
    /// signer's address in the `from` field.
    ///
    /// [`TransactionRequest`]: alloy_rpc_types_eth::TransactionRequest
    pub fn register_signer<S>(&mut self, signer: S)
    where
        S: TxSigner<Signature> + Send + Sync + 'static,
    {
        self.signers.insert(signer.address(), Arc::new(signer));
    }

    /// Register a new signer on this object, and set it as the default signer.
    /// This signer will be used to sign [`TransactionRequest`] and
    /// [`TypedTransaction`] objects that do not specify a signer address in the
    /// `from` field.
    ///
    /// [`TransactionRequest`]: alloy_rpc_types_eth::TransactionRequest
    pub fn register_default_signer<S>(&mut self, signer: S)
    where
        S: TxSigner<Signature> + Send + Sync + 'static,
    {
        self.default = signer.address();
        self.register_signer(signer);
    }

    /// Get the default signer.
    pub fn default_signer(&self) -> Arc<dyn TxSigner<Signature> + Send + Sync + 'static> {
        self.signers.get(&self.default).cloned().expect("invalid signer")
    }

    /// Get the signer for the given address.
    pub fn signer_by_address(
        &self,
        address: Address,
    ) -> Option<Arc<dyn TxSigner<Signature> + Send + Sync + 'static>> {
        self.signers.get(&address).cloned()
    }

    #[doc(alias = "sign_tx_inner")]
    async fn sign_transaction_inner(
        &self,
        sender: Address,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> alloy_signer::Result<Signature> {
        self.signer_by_address(sender)
            .ok_or_else(|| {
                alloy_signer::Error::other(format!("Missing signing credential for {}", sender))
            })?
            .sign_transaction(tx)
            .await
    }
}

impl<N> NetworkWallet<N> for EthereumWallet
where
    N: Network<UnsignedTx = TypedTransaction, TxEnvelope = TxEnvelope>,
{
    fn default_signer_address(&self) -> Address {
        self.default
    }

    fn has_signer_for(&self, address: &Address) -> bool {
        self.signers.contains_key(address)
    }

    fn signer_addresses(&self) -> impl Iterator<Item = Address> {
        self.signers.keys().copied()
    }

    #[doc(alias = "sign_tx_from")]
    async fn sign_transaction_from(
        &self,
        sender: Address,
        tx: TypedTransaction,
    ) -> alloy_signer::Result<TxEnvelope> {
        match tx {
            TypedTransaction::Legacy(mut t) => {
                let sig = self.sign_transaction_inner(sender, &mut t).await?;
                Ok(t.into_signed(sig).into())
            }
            TypedTransaction::Eip2930(mut t) => {
                let sig = self.sign_transaction_inner(sender, &mut t).await?;
                Ok(t.into_signed(sig).into())
            }
            TypedTransaction::Eip1559(mut t) => {
                let sig = self.sign_transaction_inner(sender, &mut t).await?;
                Ok(t.into_signed(sig).into())
            }
            TypedTransaction::Eip4844(mut t) => {
                let sig = self.sign_transaction_inner(sender, &mut t).await?;
                Ok(t.into_signed(sig).into())
            }
            TypedTransaction::Eip7702(mut t) => {
                let sig = self.sign_transaction_inner(sender, &mut t).await?;
                Ok(t.into_signed(sig).into())
            }
        }
    }
}

impl NetworkWallet<AnyNetwork> for EthereumWallet {
    fn default_signer_address(&self) -> Address {
        self.default
    }

    fn has_signer_for(&self, address: &Address) -> bool {
        self.signers.contains_key(address)
    }

    fn signer_addresses(&self) -> impl Iterator<Item = Address> {
        self.signers.keys().copied()
    }

    #[doc(alias = "sign_tx_from")]
    async fn sign_transaction_from(
        &self,
        sender: Address,
        tx: AnyTypedTransaction,
    ) -> alloy_signer::Result<AnyTxEnvelope> {
        match tx {
            AnyTypedTransaction::Ethereum(t) => Ok(AnyTxEnvelope::Ethereum(
                NetworkWallet::<Ethereum>::sign_transaction_from(self, sender, t).await?,
            )),
            _ => Err(alloy_signer::Error::other("cannot sign UnknownTypedTransaction")),
        }
    }
}
