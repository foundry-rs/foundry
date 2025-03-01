#![doc = include_str!("../README.md")]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/alloy.jpg",
    html_favicon_url = "https://raw.githubusercontent.com/alloy-rs/core/main/assets/favicon.ico"
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "arbitrary")]
use rand as _;

pub use alloy_trie::TrieAccount;

#[deprecated(since = "0.7.3", note = "use TrieAccount instead")]
pub use alloy_trie::TrieAccount as Account;

mod block;
pub use block::{Block, BlockBody, BlockHeader, Header};

pub mod constants;
pub use constants::{EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};

mod receipt;
pub use receipt::{
    Eip2718EncodableReceipt, Eip658Value, Receipt, ReceiptEnvelope, ReceiptWithBloom, Receipts,
    RlpDecodableReceipt, RlpEncodableReceipt, TxReceipt,
};

pub mod conditional;
pub mod proofs;

pub mod transaction;
#[cfg(feature = "kzg")]
pub use transaction::BlobTransactionValidationError;
pub use transaction::{
    SignableTransaction, Transaction, TxEip1559, TxEip2930, TxEip4844, TxEip4844Variant,
    TxEip4844WithSidecar, TxEip7702, TxEnvelope, TxLegacy, TxType, TypedTransaction,
};

pub use alloy_eips::{
    eip4844::{
        builder::{SidecarBuilder, SidecarCoder, SimpleCoder},
        utils, Blob, BlobTransactionSidecar, Bytes48,
    },
    Typed2718,
};

#[cfg(feature = "kzg")]
pub use alloy_eips::eip4844::env_settings::EnvKzgSettings;

pub use alloy_primitives::{Sealable, Sealed};

mod signed;
pub use signed::Signed;

/// Bincode-compatible serde implementations for consensus types.
///
/// `bincode` crate doesn't work well with optionally serializable serde fields, but some of the
/// consensus types require optional serialization for RPC compatibility. This module makes so that
/// all fields are serialized.
///
/// Read more: <https://github.com/bincode-org/bincode/issues/326>
#[cfg(all(feature = "serde", feature = "serde-bincode-compat"))]
pub mod serde_bincode_compat {
    pub use super::{
        block::serde_bincode_compat::*,
        transaction::{serde_bincode_compat as transaction, serde_bincode_compat::*},
    };
}
