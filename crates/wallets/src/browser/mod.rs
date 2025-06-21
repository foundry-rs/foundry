//! Browser wallet integration for Foundry.
//!
//! This module provides a minimal HTTP server that serves HTML pages
//! for wallet interaction following the Moccasin pattern.
//!
//! All browser wallets are accessed through the standard window.ethereum interface.

mod assets;
mod communication;
mod error;
mod server;
mod signer;
mod state;

#[cfg(test)]
mod tests;

pub use error::BrowserWalletError;
pub use server::BrowserWalletServer;
pub use signer::BrowserSigner;

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

/// Wallet connection information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConnection {
    pub address: Address,
    pub chain_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_name: Option<String>,
}

/// Transaction request from the CLI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRequest {
    pub id: String,
    pub from: Address,
    pub to: Option<Address>,
    pub value: Option<String>,
    pub data: Option<String>,
    pub gas: Option<String>,
    pub gas_price: Option<String>,
    pub max_fee_per_gas: Option<String>,
    pub max_priority_fee_per_gas: Option<String>,
    pub nonce: Option<u64>,
    pub chain_id: Option<u64>,
}

/// Transaction response from the browser
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResponse {
    pub id: String,
    pub hash: Option<String>,
    pub error: Option<String>,
}

/// Message signing request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignRequest {
    pub id: String,
    pub message: String,
    pub address: Address,
}

/// Message signing response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignResponse {
    pub id: String,
    pub signature: Option<String>,
    pub error: Option<String>,
}
