#[cfg(feature = "base")]
mod base;
mod envelope;
#[cfg(feature = "optimism")]
mod optimism;
mod receipt;
mod request;

pub use envelope::{FoundryTxEnvelope, FoundryTxType, FoundryTypedTx};
pub use receipt::FoundryReceiptEnvelope;
pub use request::{FoundryTransactionRequest, TempoTransactionRequest};

#[cfg(all(feature = "base", not(feature = "optimism")))]
pub use base::get_deposit_tx_parts;

#[cfg(feature = "optimism")]
pub use optimism::get_deposit_tx_parts;
