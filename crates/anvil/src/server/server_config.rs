use std::collections::HashMap;

/// Anvil server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Custom headers for HTTP responses
    pub headers: HashMap<String, String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            headers: HashMap::new(),
        }
    }
}

impl ServerConfig {
    /// Creates a new server configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a custom header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Adds multiple custom headers
    pub fn with_headers(mut self, headers: impl Into<HashMap<String, String>>) -> Self {
        self.headers.extend(headers.into());
        self
    }
} 
