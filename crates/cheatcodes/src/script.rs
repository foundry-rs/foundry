//! Broadcast intents and script wallets shared by execution and publication.

use crate::Result;
use alloy_consensus::{BlobTransactionSidecarVariant, SidecarBuilder, SimpleCoder};
use alloy_eips::eip7702::SignedAuthorization;
use alloy_network::{Ethereum, Network};
use alloy_primitives::Address;
use alloy_signer_local::PrivateKeySigner;
use foundry_common::TransactionMaybeSigned;
use foundry_wallets::{WalletSigner, wallet_multi::MultiWallet};
use parking_lot::Mutex;
use std::{collections::VecDeque, sync::Arc};

#[cfg(feature = "revm")]
mod legacy;

/// Helps collecting transactions from different forks.
#[derive(Clone, Debug)]
pub struct BroadcastableTransaction<N: Network = Ethereum> {
    /// The optional RPC URL.
    pub rpc: Option<String>,
    /// The transaction to broadcast.
    pub transaction: TransactionMaybeSigned<N>,
    /// The CREATE address captured from account state for a pre-signed transaction.
    pub contract_address: Option<Address>,
}

/// List of transactions that can be broadcasted.
pub type BroadcastableTransactions<N> = VecDeque<BroadcastableTransaction<N>>;

#[derive(Clone, Debug, Default)]
pub struct Broadcast {
    /// Address of the transaction origin
    pub new_origin: Address,
    /// Original caller
    pub original_caller: Address,
    /// Original `tx.origin`
    pub original_origin: Address,
    /// Depth of the broadcast
    pub depth: usize,
    /// Whether the prank stops by itself after the next call
    pub single_call: bool,
    /// Whether `vm.deployCode` cheatcode is used to deploy from code.
    #[cfg(feature = "revm")]
    pub deploy_from_code: bool,
}

/// Contains context for wallet management.
#[derive(Debug)]
pub struct WalletsInner {
    /// All signers in scope of the script.
    pub multi_wallet: MultiWallet,
    /// Optional signer provided as `--sender` flag.
    pub provided_sender: Option<Address>,
}

/// Cloneable wrapper around [`WalletsInner`].
#[derive(Debug, Clone)]
pub struct Wallets {
    /// Inner data.
    pub inner: Arc<Mutex<WalletsInner>>,
}

impl Wallets {
    #[expect(missing_docs)]
    pub fn new(multi_wallet: MultiWallet, provided_sender: Option<Address>) -> Self {
        Self { inner: Arc::new(Mutex::new(WalletsInner { multi_wallet, provided_sender })) }
    }

    /// Consumes [Wallets] and returns [MultiWallet].
    ///
    /// Panics if [Wallets] is still in use.
    pub fn into_multi_wallet(self) -> MultiWallet {
        Arc::into_inner(self.inner)
            .map(|m| m.into_inner().multi_wallet)
            .unwrap_or_else(|| panic!("not all instances were dropped"))
    }

    /// Locks inner Mutex and adds a signer to the [MultiWallet].
    pub fn add_local_signer(&self, wallet: PrivateKeySigner) {
        self.inner.lock().multi_wallet.add_signer(WalletSigner::Local(wallet));
    }

    /// Locks inner Mutex and returns all signer addresses in the [MultiWallet].
    pub fn signers(&self) -> Result<Vec<Address>> {
        Ok(self.inner.lock().multi_wallet.signers()?.keys().copied().collect())
    }

    /// Locks inner Mutex and returns all available addresses in the [MultiWallet].
    pub fn addresses(&self) -> Vec<Address> {
        self.inner.lock().multi_wallet.available_addresses()
    }

    /// Number of signers in the [MultiWallet].
    pub fn len(&self) -> usize {
        let mut inner = self.inner.lock();
        inner.multi_wallet.signers().map_or(0, |signers| signers.len())
    }

    /// Whether the [MultiWallet] is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Returns the next valid nonce for a delegation, considering existing active delegations.
pub(super) fn next_delegation_nonce(
    active_delegations: &[SignedAuthorization],
    authority: Address,
    broadcast: &Option<Broadcast>,
    account_nonce: u64,
) -> u64 {
    match active_delegations
        .iter()
        .rfind(|auth| auth.recover_authority().is_ok_and(|recovered| recovered == authority))
    {
        Some(auth) => {
            // Increment nonce of last recorded delegation.
            auth.nonce() + 1
        }
        None => {
            // First time a delegation is added for this authority.
            if let Some(broadcast) = broadcast {
                // Increment nonce if authority is the sender of transaction.
                if broadcast.new_origin == authority {
                    return account_nonce + 1;
                }
            }
            // Return current nonce if authority is not the sender of transaction.
            account_nonce
        }
    }
}

/// Constructs the broadcast sidecar in the format selected by the execution hardfork.
pub(crate) fn build_blob_sidecar(
    blob: &[u8],
    osaka: bool,
) -> Result<BlobTransactionSidecarVariant> {
    let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(blob);
    if osaka {
        Ok(sidecar.build_7594().map_err(|error| format!("{error}"))?.into())
    } else {
        Ok(sidecar.build_4844().map_err(|error| format!("{error}"))?.into())
    }
}
