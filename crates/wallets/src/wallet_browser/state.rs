use std::sync::Arc;

use parking_lot::Mutex;
use uuid::Uuid;

use crate::wallet_browser::{
    queue::RequestQueue,
    types::{
        BrowserSignRequest, BrowserSignResponse, BrowserTransactionRequest,
        BrowserTransactionResponse, Connection,
    },
};

#[derive(Debug, Clone)]
pub(crate) struct BrowserWalletState {
    /// Current information about the wallet connection.
    connection: Arc<Mutex<Option<Connection>>>,
    /// Request/response queue for transactions.
    transactions: Arc<Mutex<RequestQueue<BrowserTransactionRequest, BrowserTransactionResponse>>>,
    /// Request/response queue for signings.
    signings: Arc<Mutex<RequestQueue<BrowserSignRequest, BrowserSignResponse>>>,
    /// Unique session token for the wallet browser instance.
    /// The CSP on the served page prevents this token from being loaded by other origins.
    session_token: String,
    /// If true, the server is running in development mode.
    /// This relaxes certain security restrictions for local development.
    ///
    /// **WARNING**: This should only be used in a development environment.
    development: bool,
}

impl BrowserWalletState {
    /// Create a new browser wallet state.
    pub fn new(session_token: String, development: bool) -> Self {
        Self {
            connection: Arc::new(Mutex::new(None)),
            transactions: Arc::new(Mutex::new(RequestQueue::new())),
            signings: Arc::new(Mutex::new(RequestQueue::new())),
            session_token,
            development,
        }
    }

    /// Get the session token.
    pub fn session_token(&self) -> &str {
        &self.session_token
    }

    /// Check if in development mode.
    /// This relaxes certain security restrictions for local development.
    ///
    /// **WARNING**: This should only be used in a development environment.
    pub fn is_development(&self) -> bool {
        self.development
    }

    /// Check if wallet is connected.
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
    pub fn add_transaction_request(&self, request: BrowserTransactionRequest) {
        self.transactions.lock().add_request(request);
    }

    /// Check if a transaction request exists.
    pub fn has_transaction_request(&self, id: &Uuid) -> bool {
        self.transactions.lock().has_request(id)
    }

    /// Read the next transaction request.
    pub fn read_next_transaction_request(&self) -> Option<BrowserTransactionRequest> {
        self.transactions.lock().read_request().cloned()
    }

    // Remove a transaction request.
    pub fn remove_transaction_request(&self, id: &Uuid) {
        self.transactions.lock().remove_request(id);
    }

    /// Add transaction response.
    pub fn add_transaction_response(&self, response: BrowserTransactionResponse) {
        let id = response.id;
        let mut transactions = self.transactions.lock();
        transactions.add_response(id, response);
        transactions.remove_request(&id);
    }

    /// Get transaction response, removing it from the queue.
    pub fn get_transaction_response(&self, id: &Uuid) -> Option<BrowserTransactionResponse> {
        self.transactions.lock().get_response(id)
    }

    /// Add a signing request.
    pub fn add_signing_request(&self, request: BrowserSignRequest) {
        self.signings.lock().add_request(request);
    }

    /// Check if a signing request exists.
    pub fn has_signing_request(&self, id: &Uuid) -> bool {
        self.signings.lock().has_request(id)
    }

    /// Read the next signing request.
    pub fn read_next_signing_request(&self) -> Option<BrowserSignRequest> {
        self.signings.lock().read_request().cloned()
    }

    /// Remove a signing request.
    pub fn remove_signing_request(&self, id: &Uuid) {
        self.signings.lock().remove_request(id);
    }

    /// Add signing response.
    pub fn add_signing_response(&self, response: BrowserSignResponse) {
        let id = response.id;
        let mut signings = self.signings.lock();
        signings.add_response(id, response);
        signings.remove_request(&id);
    }

    /// Get signing response, removing it from the queue.
    pub fn get_signing_response(&self, id: &Uuid) -> Option<BrowserSignResponse> {
        self.signings.lock().get_response(id)
    }
}
