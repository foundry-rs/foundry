use alloy_rpc_client::ClientBuilder;
use eyre::Result;
use std::{process, thread::sleep, time::Duration};
use subxt::{OnlineClient, PolkadotConfig};
use tempfile::TempDir;

const NODE_BINARY: &str = "substrate-node";
const RPC_PROXY_BINARY: &str = "eth-rpc";
const MAX_ATTEMPTS: u32 = 15;
const RPC_URL: &str = "http://127.0.0.1:8545";
const RETRY_DELAY: Duration = Duration::from_secs(1);

const WALLETS: [(&str, &str); 1] = [(
    "0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac",
    "0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133",
)];

/// Spawn and manage an instance of a compatible contracts enabled chain node.
#[allow(dead_code)]
struct ContractsNodeProcess {
    node: process::Child,
    tmp_dir: tempfile::TempDir,
}

impl Drop for ContractsNodeProcess {
    fn drop(&mut self) {
        self.kill()
    }
}

impl ContractsNodeProcess {
    async fn start() -> Result<Self> {
        let tmp_dir = TempDir::with_prefix("cargo-contract.cli.test.node")?;

        let mut node = process::Command::new(NODE_BINARY)
            .env("RUST_LOG", "error")
            .arg("--dev")
            .arg(format!("--base-path={}", tmp_dir.path().to_string_lossy()))
            .spawn()?;
        // wait for rpc to be initialized
        let mut attempts = 1;
        loop {
            sleep(RETRY_DELAY);
            tracing::debug!(
                "Connecting to contracts enabled node, attempt {}/{}",
                attempts,
                MAX_ATTEMPTS
            );
            match OnlineClient::<PolkadotConfig>::new().await {
                Result::Ok(_) => return Ok(Self { node, tmp_dir }),
                Err(err) => {
                    if attempts < MAX_ATTEMPTS {
                        attempts += 1;
                        continue;
                    }
                    let err = eyre::eyre!(
                        "Failed to connect to node rpc after {} attempts: {}",
                        attempts,
                        err
                    );
                    tracing::error!("{}", err);
                    node.kill()?;
                    return Err(err);
                }
            }
        }
    }

    fn kill(&mut self) {
        tracing::debug!("Killing contracts node process {}", self.node.id());
        if let Err(err) = self.node.kill() {
            tracing::error!("Error killing contracts node process {}: {}", self.node.id(), err)
        }
    }
}

/// Spawn and manage an instance of an ethereum RPC proxy node.
struct RpcProxyProcess(process::Child);

impl Drop for RpcProxyProcess {
    fn drop(&mut self) {
        self.kill()
    }
}

impl RpcProxyProcess {
    async fn start() -> Result<Self> {
        let mut rpc_proxy = process::Command::new(RPC_PROXY_BINARY)
            .env("RUST_LOG", "error")
            .arg("--dev")
            .spawn()?;

        let client = ClientBuilder::default().connect(RPC_URL).await?;

        let mut attempts = 1;
        loop {
            sleep(RETRY_DELAY);
            match client.request_noparams::<String>("eth_chainId").await {
                Result::Ok(_) => {
                    return Ok(Self(rpc_proxy));
                }
                Err(err) => {
                    if attempts < MAX_ATTEMPTS {
                        attempts += 1;
                        continue;
                    }

                    let err = eyre::eyre!(
                        "Failed to connect to RPC proxy after {} attempts: {}",
                        MAX_ATTEMPTS,
                        err
                    );
                    tracing::error!("{}", err);
                    rpc_proxy.kill()?;
                    return Err(err);
                }
            }
        }
    }

    fn kill(&mut self) {
        tracing::debug!("Killing RPC proxy process {}", self.0.id());
        if let Err(err) = self.0.kill() {
            tracing::error!("Error killing RPC proxy process {}: {}", self.0.id(), err)
        }
    }
}

/// `PolkadotHubNode` combines a `substrate-node` with an Ethereum RPC proxy to enable
/// Ethereum-compatible transactions in CI tests.
///
/// Before using it, make sure both `substrate-node` and the Ethereum RPC proxy are installed:
///
/// ```bash
/// git clone https://github.com/paritytech/polkadot-sdk
/// cd polkadot-sdk
/// cargo build --release --bin substrate-node
///
/// cargo install pallet-revive-eth-rpc
/// ```
///
/// Ensure that both binaries are available in your system's PATH and are version-compatible.
#[allow(dead_code)]
pub struct PolkadotNode {
    node: ContractsNodeProcess,
    rpc_proxy: RpcProxyProcess,
}

impl PolkadotNode {
    pub async fn start() -> Result<Self> {
        let node = ContractsNodeProcess::start().await?;
        let rpc_proxy = RpcProxyProcess::start().await?;
        Ok(Self { node, rpc_proxy })
    }

    pub fn http_endpoint() -> &'static str {
        RPC_URL
    }

    pub fn dev_accounts() -> impl Iterator<Item = (&'static str, &'static str)> {
        WALLETS.iter().copied()
    }
}
