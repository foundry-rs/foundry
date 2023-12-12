use crate::eth::{backend::fork::ClientFork, EthApi};

pub const HELLO_WORLD: &str = "hello world";

#[derive(Clone)]
pub struct EngineApi {
    pub eth_api: EthApi
}

impl EngineApi {
    /// Creates a new instance
    #[allow(clippy::too_many_arguments)]
    pub fn new(eth_api: EthApi) -> Self {
        Self { eth_api }
    }

    pub async fn execute(&self) {
        self.hello_world();
    }

    pub fn hello_world(&self) -> Result<String,String> {
        print!("hello world");
        Ok(HELLO_WORLD.to_string())
    }

    pub fn get_fork(&self) -> Option<ClientFork> {
        None
    }
}