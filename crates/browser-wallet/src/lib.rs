//! # Browser Wallet Support for Foundry
//!
//! This crate implements browser wallet integration following:
//! - [EIP-1193](https://eips.ethereum.org/EIPS/eip-1193): Ethereum Provider JavaScript API
//! - [EIP-712](https://eips.ethereum.org/EIPS/eip-712): Typed structured data hashing and signing
//! - JSON-RPC 2.0 for communication protocol
//!
//! ## Architecture
//!
//! The implementation uses a local HTTP server to bridge between CLI and browser:
//! 1. CLI starts local server and opens browser
//! 2. Browser connects to MetaMask/ injected wallets via window.ethereum
//! 3. Transactions are queued and processed asynchronously
//! 4. Results are returned to CLI via polling
//!
//! ## Standards and References
//!
//! This implementation adheres to the following standards:
//! - **EIP-1193**: Defines the JavaScript Ethereum Provider API that browser wallets expose
//! - **EIP-712**: Specifies typed structured data hashing and signing
//! - **JSON-RPC 2.0**: Communication protocol between the CLI and browser

mod assets;
mod error;
mod server;
mod signer;
mod state;

pub use error::BrowserWalletError;
pub use server::BrowserWalletServer;
pub use signer::BrowserSigner;

use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, Bytes, ChainId, B256};
use alloy_rpc_types::TransactionRequest;
use serde::{Deserialize, Serialize};

/// Wallet connection information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConnection {
    pub address: Address,
    pub chain_id: ChainId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_name: Option<String>,
}

/// Browser-specific transaction wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTransaction {
    /// Unique ID for tracking in the browser
    pub id: String,
    /// Standard Alloy transaction request
    #[serde(flatten)]
    pub request: TransactionRequest,
}

/// Transaction response from the browser
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResponse {
    pub id: String,
    pub hash: Option<B256>,
    pub error: Option<String>,
}

/// Type of signature request
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignType {
    PersonalSign,
    SignTypedData,
}

/// Message signing request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignRequest {
    pub id: String,
    pub message: String,
    pub address: Address,
    #[serde(rename = "type")]
    pub sign_type: SignType,
}

/// Message signing response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignResponse {
    pub id: String,
    pub signature: Option<Bytes>,
    pub error: Option<String>,
}

/// Typed data signing request following EIP-712
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedDataRequest {
    pub id: String,
    pub address: Address,
    pub typed_data: TypedData,
}

/// Standard EIP-1193 provider interface
/// Reference: https://eips.ethereum.org/EIPS/eip-1193
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum EthereumRequest {
    #[serde(rename = "eth_requestAccounts")]
    RequestAccounts,

    #[serde(rename = "eth_sendTransaction")]
    SendTransaction([TransactionRequest; 1]),

    #[serde(rename = "personal_sign")]
    PersonalSign(String, Address),

    #[serde(rename = "eth_signTypedData_v4")]
    SignTypedData(Address, TypedData),
}

/// Response wrapper for browser communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
