mod abi;
mod anvil;
// mod anvil_api;
// mod api;
// mod fork;
// mod ganache;
mod gas;
mod genesis;
mod geth;
// mod ipc;
mod logs;
// mod proof;
// mod pubsub;
// // mod revert; // TODO uncomment <https://github.com/gakonst/ethers-rs/issues/2186>
mod otterscan;
// mod sign;
// mod traces;
mod transaction;
mod txpool;
pub mod utils;
// mod wsapi;

#[allow(unused)]
pub(crate) fn init_tracing() {
    let _ = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

fn main() {}
