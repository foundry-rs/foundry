use std::collections::{HashMap, VecDeque};

use uuid::Uuid;

use crate::wallet_browser::types::{BrowserSignRequest, BrowserTransactionRequest};

#[derive(Debug)]
pub(crate) struct RequestQueue<Req, Res> {
    /// Pending requests from CLI to browser
    requests: VecDeque<Req>,
    /// Responses from browser indexed by request ID
    responses: HashMap<Uuid, Res>,
}

impl<Req, Res> Default for RequestQueue<Req, Res> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Req, Res> RequestQueue<Req, Res> {
    /// Create a new request queue.
    pub fn new() -> Self {
        Self { requests: VecDeque::new(), responses: HashMap::new() }
    }

    /// Add a new request to the queue.
    pub fn add_request(&mut self, request: Req) {
        self.requests.push_back(request);
    }

    /// Check if the queue contains any pending requests matching the given ID.
    pub fn has_request(&self, id: &Uuid) -> bool
    where
        Req: HasId,
    {
        self.requests.iter().any(|r| r.id() == id)
    }

    /// Read the next request from the queue without removing it.
    pub fn read_request(&self) -> Option<&Req> {
        self.requests.front()
    }

    /// Remove a request by its ID.
    pub fn remove_request(&mut self, id: &Uuid) -> Option<Req>
    where
        Req: HasId,
    {
        if let Some(pos) = self.requests.iter().position(|r| r.id() == id) {
            self.requests.remove(pos)
        } else {
            None
        }
    }

    /// Add a response to the queue.
    pub fn add_response(&mut self, id: Uuid, response: Res) {
        self.responses.insert(id, response);
    }

    /// Get a response by its ID, removing it from the queue.
    pub fn get_response(&mut self, id: &Uuid) -> Option<Res> {
        self.responses.remove(id)
    }
}

pub(crate) trait HasId {
    fn id(&self) -> &Uuid;
}

impl HasId for BrowserTransactionRequest {
    fn id(&self) -> &Uuid {
        &self.id
    }
}

impl HasId for BrowserSignRequest {
    fn id(&self) -> &Uuid {
        &self.id
    }
}
