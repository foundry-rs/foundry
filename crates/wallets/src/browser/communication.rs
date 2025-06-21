use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::Mutex;

/// Shared communication state between server and signer
#[derive(Clone, Debug)]
pub struct CommunicationState {
    /// Queue of transaction requests from CLI to browser
    pub transaction_request_queue: Arc<Mutex<VecDeque<TransactionRequest>>>,
    /// Queue of transaction responses from browser to CLI
    pub transaction_response_queue: Arc<Mutex<VecDeque<TransactionResponse>>>,
    /// Queue of signing requests from CLI to browser
    pub signing_request_queue: Arc<Mutex<VecDeque<SigningRequest>>>,
    /// Queue of signing responses from browser to CLI
    pub signing_response_queue: Arc<Mutex<VecDeque<SigningResponse>>>,
    /// Current connected address
    pub connected_address: Arc<Mutex<Option<String>>>,
    /// Network details
    pub network_details: Arc<Mutex<NetworkDetails>>,
}

impl CommunicationState {
    pub fn new() -> Self {
        Self {
            transaction_request_queue: Arc::new(Mutex::new(VecDeque::new())),
            transaction_response_queue: Arc::new(Mutex::new(VecDeque::new())),
            signing_request_queue: Arc::new(Mutex::new(VecDeque::new())),
            signing_response_queue: Arc::new(Mutex::new(VecDeque::new())),
            connected_address: Arc::new(Mutex::new(None)),
            network_details: Arc::new(Mutex::new(NetworkDetails::default())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRequest {
    pub id: String,
    pub from: String,
    pub to: Option<String>,
    pub value: String,
    pub data: Option<String>,
    pub gas: Option<String>,
    pub gas_price: Option<String>,
    pub max_fee_per_gas: Option<String>,
    pub max_priority_fee_per_gas: Option<String>,
    pub nonce: Option<u64>,
    pub chain_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResponse {
    pub id: String,
    pub status: String, // "success" or "error"
    pub hash: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkDetails {
    pub chain_id: u64,
    pub rpc_url: String,
    pub network_name: String,
}

/// Represents a message signing request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningRequest {
    pub id: String,
    pub from: String,
    #[serde(rename = "type")]
    pub signing_type: SigningType,
    pub data: String,
}

/// Type of signing request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SigningType {
    PersonalSign,
    SignTypedData,
}

/// Response from a signing request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningResponse {
    pub id: String,
    pub status: String, // "success" or "error"
    pub signature: Option<String>,
    pub error: Option<String>,
}