use alloy_consensus::{Sealed, SignableTransaction};
use alloy_network::{Ethereum, EthereumWallet, NetworkWallet, TxSigner};
use alloy_primitives::{Address, Signature};
use alloy_signer::{Error, Result};

use crate::{FoundryNetwork, FoundryTxEnvelope, FoundryTypedTx};

impl NetworkWallet<FoundryNetwork> for EthereumWallet {
    fn default_signer_address(&self) -> Address {
        NetworkWallet::<Ethereum>::default_signer_address(self)
    }

    fn has_signer_for(&self, address: &Address) -> bool {
        NetworkWallet::<Ethereum>::has_signer_for(self, address)
    }

    fn signer_addresses(&self) -> impl Iterator<Item = Address> {
        NetworkWallet::<Ethereum>::signer_addresses(self)
    }

    async fn sign_transaction_from(
        &self,
        sender: Address,
        tx: FoundryTypedTx,
    ) -> Result<FoundryTxEnvelope> {
        match tx {
            FoundryTypedTx::Legacy(mut tx) => {
                let sig = sign_with_wallet(self, sender, &mut tx).await?;
                Ok(FoundryTxEnvelope::Legacy(tx.into_signed(sig)))
            }
            FoundryTypedTx::Eip2930(mut tx) => {
                let sig = sign_with_wallet(self, sender, &mut tx).await?;
                Ok(FoundryTxEnvelope::Eip2930(tx.into_signed(sig)))
            }
            FoundryTypedTx::Eip1559(mut tx) => {
                let sig = sign_with_wallet(self, sender, &mut tx).await?;
                Ok(FoundryTxEnvelope::Eip1559(tx.into_signed(sig)))
            }
            FoundryTypedTx::Eip4844(mut tx) => {
                let sig = sign_with_wallet(self, sender, &mut tx).await?;
                Ok(FoundryTxEnvelope::Eip4844(tx.into_signed(sig)))
            }
            FoundryTypedTx::Eip7702(mut tx) => {
                let sig = sign_with_wallet(self, sender, &mut tx).await?;
                Ok(FoundryTxEnvelope::Eip7702(tx.into_signed(sig)))
            }
            FoundryTypedTx::Deposit(tx) => {
                // Deposit transactions don't require signing
                Ok(FoundryTxEnvelope::Deposit(Sealed::new(tx)))
            }
            FoundryTypedTx::Tempo(mut tx) => {
                let sig = sign_with_wallet(self, sender, &mut tx).await?;
                Ok(FoundryTxEnvelope::Tempo(tx.into_signed(sig.into())))
            }
        }
    }
}

/// Helper function to sign a transaction using the wallet's signer for the given sender address.
async fn sign_with_wallet(
    wallet: &EthereumWallet,
    sender: Address,
    tx: &mut dyn SignableTransaction<Signature>,
) -> Result<Signature> {
    let signer = wallet
        .signer_by_address(sender)
        .ok_or_else(|| Error::other(format!("Signer not found for sender {sender}")))?;
    TxSigner::sign_transaction(&signer, tx).await
}
