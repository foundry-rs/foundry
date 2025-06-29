use crate::{BrowserTransaction, SignRequest, SignResponse, TransactionResponse};
use parking_lot::Mutex;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Instant,
};

/// Generic request/response queue for browser wallet operations
#[derive(Debug)]
pub struct RequestQueue<Req, Res> {
    /// Pending requests from CLI to browser
    requests: VecDeque<Req>,
    /// Responses from browser indexed by request ID
    responses: HashMap<String, Res>,
}

impl<Req, Res> Default for RequestQueue<Req, Res> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Req, Res> RequestQueue<Req, Res> {
    pub fn new() -> Self {
        Self { requests: VecDeque::new(), responses: HashMap::new() }
    }

    pub fn add_request(&mut self, request: Req) {
        self.requests.push_back(request);
    }

    pub fn get_pending(&self) -> Option<&Req> {
        self.requests.front()
    }

    pub fn remove_request(&mut self, id: &str) -> Option<Req>
    where
        Req: HasId,
    {
        if let Some(pos) = self.requests.iter().position(|r| r.id() == id) {
            self.requests.remove(pos)
        } else {
            None
        }
    }

    pub fn add_response(&mut self, id: String, response: Res) {
        self.responses.insert(id, response);
    }

    pub fn get_response(&mut self, id: &str) -> Option<Res> {
        self.responses.remove(id)
    }
}

/// Trait for types that have an ID
pub trait HasId {
    fn id(&self) -> &str;
}

impl HasId for BrowserTransaction {
    fn id(&self) -> &str {
        &self.id
    }
}

impl HasId for SignRequest {
    fn id(&self) -> &str {
        &self.id
    }
}

/// Connection information
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    pub address: Option<String>,
    pub chain_id: Option<u64>,
}

/// Simplified browser wallet state
#[derive(Debug, Clone)]
pub struct BrowserWalletState {
    /// Transaction request/response queue
    pub transactions: Arc<Mutex<RequestQueue<BrowserTransaction, TransactionResponse>>>,
    /// Signing request/response queue
    pub signing: Arc<Mutex<RequestQueue<SignRequest, SignResponse>>>,
    /// Current connection info
    pub connection: Arc<Mutex<ConnectionInfo>>,
    /// Last heartbeat timestamp
    pub last_heartbeat: Arc<Mutex<Instant>>,
}

impl Default for BrowserWalletState {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserWalletState {
    pub fn new() -> Self {
        Self {
            transactions: Arc::new(Mutex::new(RequestQueue::new())),
            signing: Arc::new(Mutex::new(RequestQueue::new())),
            connection: Arc::new(Mutex::new(ConnectionInfo::default())),
            last_heartbeat: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Add a transaction request
    pub fn add_transaction_request(&self, request: BrowserTransaction) {
        self.transactions.lock().add_request(request);
    }

    /// Get pending transaction
    pub fn get_pending_transaction(&self) -> Option<BrowserTransaction> {
        self.transactions.lock().get_pending().cloned()
    }

    /// Remove transaction request
    pub fn remove_transaction_request(&self, id: &str) {
        self.transactions.lock().remove_request(id);
    }

    /// Add transaction response
    pub fn add_transaction_response(&self, response: TransactionResponse) {
        let id = response.id.clone();
        // Add to responses first (before removing from queue to avoid race)
        self.transactions.lock().add_response(id.clone(), response);
        // Then remove from request queue
        self.remove_transaction_request(&id);
    }

    /// Get transaction response
    pub fn get_transaction_response(&self, id: &str) -> Option<TransactionResponse> {
        self.transactions.lock().get_response(id)
    }

    /// Add a signing request
    pub fn add_signing_request(&self, request: SignRequest) {
        self.signing.lock().add_request(request);
    }

    /// Get pending signing request
    pub fn get_pending_signing(&self) -> Option<SignRequest> {
        self.signing.lock().get_pending().cloned()
    }

    /// Remove signing request
    pub fn remove_signing_request(&self, id: &str) {
        self.signing.lock().remove_request(id);
    }

    /// Add signing response
    pub fn add_signing_response(&self, response: SignResponse) {
        let id = response.id.clone();
        // Add to responses first (before removing from queue to avoid race)
        self.signing.lock().add_response(id.clone(), response);
        // Then remove from request queue
        self.remove_signing_request(&id);
    }

    /// Get signing response
    pub fn get_signing_response(&self, id: &str) -> Option<SignResponse> {
        self.signing.lock().get_response(id)
    }

    /// Update heartbeat
    pub fn update_heartbeat(&self) {
        *self.last_heartbeat.lock() = Instant::now();
    }

    /// Check if wallet is connected
    pub fn is_connected(&self) -> bool {
        self.connection.lock().address.is_some()
    }

    /// Set connected address
    pub fn set_connected_address(&self, address: Option<String>) {
        let mut conn = self.connection.lock();
        conn.address = address;
        // Clear chain ID if disconnecting
        if conn.address.is_none() {
            conn.chain_id = None;
        }
    }

    /// Set connected chain ID
    pub fn set_connected_chain_id(&self, chain_id: Option<u64>) {
        self.connection.lock().chain_id = chain_id;
    }

    /// Get connected address
    pub fn get_connected_address(&self) -> Option<String> {
        self.connection.lock().address.clone()
    }

    /// Get connected chain ID
    pub fn get_connected_chain_id(&self) -> Option<u64> {
        self.connection.lock().chain_id
    }
}
