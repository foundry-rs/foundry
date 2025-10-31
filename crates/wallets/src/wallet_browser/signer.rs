use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_consensus::SignableTransaction;
use alloy_network::TxSigner;
use alloy_primitives::{Address, B256, ChainId};
use alloy_rpc_types::TransactionRequest;
use alloy_signer::{Result, Signature, Signer};
use async_trait::async_trait;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::wallet_browser::{
    server::BrowserWalletServer,
    types::{BrowserTransaction, Connection},
};

#[derive(Clone, Debug)]
pub struct BrowserSigner {
    server: Arc<Mutex<BrowserWalletServer>>,
    address: Address,
    chain_id: ChainId,
}

impl BrowserSigner {
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
            if let Some(Connection(address, chain_id)) = server.get_connection() {
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
        tx_request: TransactionRequest,
    ) -> Result<B256> {
        if let Some(from) = tx_request.from
            && from != self.address
        {
            return Err(alloy_signer::Error::other(
                "Transaction `from` address does not match connected wallet address",
            ));
        }

        if let Some(chain_id) = tx_request.chain_id
            && chain_id != self.chain_id
        {
            return Err(alloy_signer::Error::other(
                "Transaction `chainId` does not match connected wallet chain ID",
            ));
        }

        let request = BrowserTransaction { id: Uuid::new_v4(), request: tx_request };

        let server = self.server.lock().await;
        let tx_hash =
            server.request_transaction(request).await.map_err(alloy_signer::Error::other)?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(tx_hash)
    }
}

#[async_trait]
impl Signer for BrowserSigner {
    async fn sign_hash(&self, _hash: &B256) -> Result<Signature> {
        Err(alloy_signer::Error::other(
            "Browser wallets sign and send transactions in one step. Use eth_sendTransaction instead.",
        ))
    }

    fn address(&self) -> Address {
        self.address
    }

    fn chain_id(&self) -> Option<ChainId> {
        Some(self.chain_id)
    }

    fn set_chain_id(&mut self, chain_id: Option<ChainId>) {
        if let Some(id) = chain_id {
            self.chain_id = id;
        }
    }
}

#[async_trait]
impl TxSigner<Signature> for BrowserSigner {
    fn address(&self) -> Address {
        self.address
    }

    async fn sign_transaction(
        &self,
        _tx: &mut dyn SignableTransaction<Signature>,
    ) -> Result<Signature> {
        Err(alloy_signer::Error::other("Use send_transaction_via_browser for browser wallets"))
    }
}

impl Drop for BrowserSigner {
    fn drop(&mut self) {
        let server = self.server.clone();

        tokio::spawn(async move {
            let mut server = server.lock().await;
            let _ = server.stop().await;
        });
    }
}
