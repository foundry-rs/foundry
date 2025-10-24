use std::collections::{HashMap, VecDeque};

use uuid::Uuid;

use crate::wallet_browser::types::BrowserTransaction;

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
    pub fn new() -> Self {
        Self { requests: VecDeque::new(), responses: HashMap::new() }
    }

    pub fn add_request(&mut self, request: Req) {
        self.requests.push_back(request);
    }

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

    pub fn add_response(&mut self, id: Uuid, response: Res) {
        self.responses.insert(id, response);
    }

    pub fn get_response(&mut self, id: &Uuid) -> Option<Res> {
        self.responses.remove(id)
    }

    pub fn get_pending(&self) -> Option<&Req> {
        self.requests.front()
    }
}

pub(crate) trait HasId {
    fn id(&self) -> &Uuid;
}

impl HasId for BrowserTransaction {
    fn id(&self) -> &Uuid {
        &self.id
    }
}
