use std::sync::Arc;

use parking_lot::Mutex;

use crate::wallet_browser::{
    queue::RequestQueue,
    types::{BrowserTransaction, TransactionResponse},
};

/// Current connection information
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    pub address: Option<String>,
    pub chain_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct BrowserWalletState {
    /// Current information about the wallet connection
    pub connection: Arc<Mutex<ConnectionInfo>>,
    /// Request/response queue for transactions
    pub transactions: Arc<Mutex<RequestQueue<BrowserTransaction, TransactionResponse>>>,
}

impl Default for BrowserWalletState {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserWalletState {
    pub fn new() -> Self {
        Self {
            connection: Arc::new(Mutex::new(ConnectionInfo::default())),
            transactions: Arc::new(Mutex::new(RequestQueue::new())),
        }
    }

    /// Check if wallet is connected
    pub fn is_connected(&self) -> bool {
        self.connection.lock().address.is_some()
    }

    /// Set connected address.
    pub fn set_connected_address(&self, address: Option<String>) {
        let mut connection = self.connection.lock();
        connection.address = address;

        // If disconnecting, clear chain ID as well
        if connection.address.is_none() {
            connection.chain_id = None;
        }
    }

    /// Set connected chain ID.
    pub fn set_connected_chain_id(&self, chain_id: Option<u64>) {
        self.connection.lock().chain_id = chain_id;
    }

    /// Get connected address.
    pub fn get_connected_address(&self) -> Option<String> {
        self.connection.lock().address.clone()
    }

    /// Get connected chain ID.
    pub fn get_connected_chain_id(&self) -> Option<u64> {
        self.connection.lock().chain_id
    }

    /// Add a transaction request.
    pub fn add_transaction_request(&self, request: BrowserTransaction) {
        self.transactions.lock().add_request(request);
    }

    /// Get pending transaction.
    pub fn get_pending_transaction(&self) -> Option<BrowserTransaction> {
        self.transactions.lock().get_pending().cloned()
    }

    /// Remove transaction request.
    pub fn remove_transaction_request(&self, id: &str) {
        self.transactions.lock().remove_request(id);
    }

    /// Add transaction response.
    pub fn add_transaction_response(&self, response: TransactionResponse) {
        let id = response.id.clone();
        self.transactions.lock().add_response(id.clone(), response);
        self.remove_transaction_request(&id);
    }

    /// Get transaction response.
    pub fn get_transaction_response(&self, id: &str) -> Option<TransactionResponse> {
        self.transactions.lock().get_response(id)
    }
}
