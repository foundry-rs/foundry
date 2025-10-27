use std::sync::Arc;

use parking_lot::Mutex;
use uuid::Uuid;

use crate::wallet_browser::{
    queue::RequestQueue,
    types::{BrowserTransaction, Connection, TransactionResponse},
};

#[derive(Debug, Clone)]
pub(crate) struct BrowserWalletState {
    /// Current information about the wallet connection
    pub connection: Arc<Mutex<Option<Connection>>>,
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
            connection: Arc::new(Mutex::new(None)),
            transactions: Arc::new(Mutex::new(RequestQueue::new())),
        }
    }

    /// Check if wallet is connected
    pub fn is_connected(&self) -> bool {
        self.connection.lock().is_some()
    }

    /// Get current connection information.
    pub fn get_connection(&self) -> Option<Connection> {
        *self.connection.lock()
    }

    /// Set connection information.
    pub fn set_connection(&self, connection: Option<Connection>) {
        *self.connection.lock() = connection;
    }

    /// Add a transaction request.
    pub fn add_transaction_request(&self, request: BrowserTransaction) {
        self.transactions.lock().add_request(request);
    }

    /// Check if a transaction request exists.
    pub fn has_transaction_request(&self, id: &Uuid) -> bool {
        self.transactions.lock().has_request(id)
    }

    /// Get next transaction request.
    pub fn get_next_transaction_request(&self) -> Option<BrowserTransaction> {
        self.transactions.lock().get_request().cloned()
    }

    // Remove a transaction request.
    pub fn remove_transaction_request(&self, id: &Uuid) {
        self.transactions.lock().remove_request(id);
    }

    /// Add transaction response.
    pub fn add_transaction_response(&self, response: TransactionResponse) {
        let id = response.id;
        self.transactions.lock().add_response(id, response);
        self.transactions.lock().remove_request(&id);
    }

    /// Get transaction response.
    pub fn get_transaction_response(&self, id: &Uuid) -> Option<TransactionResponse> {
        self.transactions.lock().get_response(id)
    }
}
