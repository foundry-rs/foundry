use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::{Address, B256, ChainId};
use alloy_signer::Result;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::wallet_browser::{
    server::BrowserWalletServer,
    types::{BrowserTransactionRequest, Connection},
};

#[derive(Clone, Debug)]
pub struct BrowserSigner<N: Network> {
    server: Arc<Mutex<BrowserWalletServer<N>>>,
    address: Address,
    chain_id: ChainId,
}

impl<N: Network> BrowserSigner<N> {
    pub async fn new(
        port: u16,
        open_browser: bool,
        timeout: Duration,
        development: bool,
    ) -> Result<Self> {
        let mut server = BrowserWalletServer::new(port, open_browser, timeout, development);

        server.start().await.map_err(alloy_signer::Error::other)?;

        let _ = sh_warn!("Browser wallet is still in early development. Use with caution!");
        let _ = sh_println!("Opening browser for wallet connection...");
        let _ = sh_println!("Waiting for wallet connection...");

        let start = Instant::now();

        loop {
            if let Some(Connection { address, chain_id }) = server.get_connection().await {
                let _ = sh_println!("Wallet connected: {}", address);
                let _ = sh_println!("Chain ID: {}", chain_id);

                return Ok(Self { server: Arc::new(Mutex::new(server)), address, chain_id });
            }

            if start.elapsed() > timeout {
                return Err(alloy_signer::Error::other("Wallet connection timeout"));
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// Send a transaction through the browser wallet.
    pub async fn send_transaction_via_browser(
        &self,
        tx_request: N::TransactionRequest,
    ) -> Result<B256> {
        if let Some(from) = tx_request.from()
            && from != self.address
        {
            return Err(alloy_signer::Error::other(
                "Transaction `from` address does not match connected wallet address",
            ));
        }

        if let Some(chain_id) = tx_request.chain_id()
            && chain_id != self.chain_id
        {
            return Err(alloy_signer::Error::other(
                "Transaction `chainId` does not match connected wallet chain ID",
            ));
        }

        let request = BrowserTransactionRequest { id: Uuid::new_v4(), request: tx_request };

        let server = self.server.lock().await;
        let tx_hash =
            server.request_transaction(request).await.map_err(alloy_signer::Error::other)?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(tx_hash)
    }

    pub const fn address(&self) -> Address {
        self.address
    }
}

impl<N: Network> Drop for BrowserSigner<N> {
    fn drop(&mut self) {
        let server = self.server.clone();

        tokio::spawn(async move {
            let mut server = server.lock().await;
            let _ = server.stop().await;
        });
    }
}
