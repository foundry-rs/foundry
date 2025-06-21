use super::communication::{
    CommunicationState, SigningRequest, SigningResponse, TransactionRequest, TransactionResponse,
};
use std::time::{Duration, Instant};

/// Manages the browser wallet state and communication
#[derive(Debug)]
pub struct BrowserWalletState {
    pub communication: CommunicationState,
    pub last_heartbeat: Instant,
}

impl BrowserWalletState {
    pub fn new() -> Self {
        Self {
            communication: CommunicationState::new(),
            last_heartbeat: Instant::now(),
        }
    }

    /// Add a transaction request to the queue
    pub fn add_transaction_request(&self, request: TransactionRequest) {
        let mut queue = self.communication.transaction_request_queue.lock();
        queue.push_back(request);
    }

    /// Get the next pending transaction request
    pub fn get_pending_transaction(&self) -> Option<TransactionRequest> {
        let queue = self.communication.transaction_request_queue.lock();
        queue.front().cloned()
    }

    /// Remove a transaction request by ID
    pub fn remove_transaction_request(&self, id: &str) {
        let mut queue = self.communication.transaction_request_queue.lock();
        queue.retain(|tx| tx.id != id);
    }

    /// Add a transaction response
    pub fn add_transaction_response(&self, response: TransactionResponse) {
        // Remove from request queue
        self.remove_transaction_request(&response.id);
        
        // Add to response queue
        let mut queue = self.communication.transaction_response_queue.lock();
        queue.push_back(response);
    }

    /// Get a transaction response by ID
    pub fn get_transaction_response(&self, id: &str) -> Option<TransactionResponse> {
        let mut queue = self.communication.transaction_response_queue.lock();
        if let Some(pos) = queue.iter().position(|r| r.id == id) {
            queue.remove(pos)
        } else {
            None
        }
    }

    /// Update heartbeat timestamp
    pub fn update_heartbeat(&self) {
        // Note: we'd need to make last_heartbeat Arc<Mutex<>> for this to work properly
        // For now, this is a placeholder
    }

    /// Check if the server is still alive (heartbeat within last 10 seconds)
    pub fn is_alive(&self) -> bool {
        self.last_heartbeat.elapsed() < Duration::from_secs(10)
    }

    /// Set the connected address
    pub fn set_connected_address(&self, address: Option<String>) {
        *self.communication.connected_address.lock() = address;
    }

    /// Get the connected address
    pub fn get_connected_address(&self) -> Option<String> {
        self.communication.connected_address.lock().clone()
    }
    
    /// Add a signing request to the queue
    pub fn add_signing_request(&self, request: SigningRequest) {
        let mut queue = self.communication.signing_request_queue.lock();
        queue.push_back(request);
    }
    
    /// Get the next pending signing request
    pub fn get_pending_signing(&self) -> Option<SigningRequest> {
        let queue = self.communication.signing_request_queue.lock();
        queue.front().cloned()
    }
    
    /// Remove a signing request by ID
    pub fn remove_signing_request(&self, id: &str) {
        let mut queue = self.communication.signing_request_queue.lock();
        queue.retain(|req| req.id != id);
    }
    
    /// Add a signing response
    pub fn add_signing_response(&self, response: SigningResponse) {
        // Remove from request queue
        self.remove_signing_request(&response.id);
        
        // Add to response queue
        let mut queue = self.communication.signing_response_queue.lock();
        queue.push_back(response);
    }
    
    /// Get a signing response by ID
    pub fn get_signing_response(&self, id: &str) -> Option<SigningResponse> {
        let mut queue = self.communication.signing_response_queue.lock();
        if let Some(pos) = queue.iter().position(|r| r.id == id) {
            queue.remove(pos)
        } else {
            None
        }
    }
}