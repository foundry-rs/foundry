//! Extended APIs for the provider module.

#[cfg(feature = "admin-api")]
mod admin;
#[cfg(feature = "admin-api")]
pub use admin::AdminApi;

#[cfg(feature = "anvil-api")]
mod anvil;
#[cfg(feature = "anvil-api")]
pub use anvil::AnvilApi;

#[cfg(feature = "engine-api")]
mod engine;
#[cfg(feature = "engine-api")]
pub use engine::EngineApi;

#[cfg(feature = "debug-api")]
mod debug;
#[cfg(feature = "debug-api")]
pub use debug::DebugApi;

#[cfg(feature = "net-api")]
mod net;
#[cfg(feature = "net-api")]
pub use net::NetApi;

#[cfg(feature = "trace-api")]
mod trace;
#[cfg(feature = "trace-api")]
pub use trace::{TraceApi, TraceCallList};

#[cfg(feature = "rpc-api")]
mod rpc;
#[cfg(feature = "rpc-api")]
pub use rpc::RpcApi;

#[cfg(feature = "txpool-api")]
mod txpool;
#[cfg(feature = "txpool-api")]
pub use txpool::TxPoolApi;

#[cfg(feature = "erc4337-api")]
mod erc4337;
#[cfg(feature = "erc4337-api")]
pub use erc4337::Erc4337Api;

#[cfg(test)]
pub(crate) mod test {
    #[allow(dead_code)] // dead only when all features off
    /// Run the given function only if we are in a CI environment.
    pub(crate) async fn async_ci_only<F, Fut>(f: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        if ci_info::is_ci() {
            f().await;
        }
    }
}
