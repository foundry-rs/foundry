use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_network::TxSigner;
use alloy_primitives::{Address, B256, ChainId, hex};
use alloy_rpc_types::TransactionRequest;
use alloy_signer::{Result, Signature, Signer, SignerSync};
use alloy_sol_types::{Eip712Domain, SolStruct};
use async_trait::async_trait;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::wallet_browser::{
    server::BrowserWalletServer,
    types::{BrowserSignRequest, BrowserTransactionRequest, Connection, SignRequest, SignType},
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

        let request = BrowserTransactionRequest { id: Uuid::new_v4(), request: tx_request };

        let server = self.server.lock().await;
        let tx_hash =
            server.request_transaction(request).await.map_err(alloy_signer::Error::other)?;

        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(tx_hash)
    }
}

impl SignerSync for BrowserSigner {
    fn sign_hash_sync(&self, _hash: &B256) -> Result<Signature> {
        Err(alloy_signer::Error::other(
            "Browser wallets cannot sign raw hashes. Use sign_message or send_transaction instead.",
        ))
    }

    fn sign_message_sync(&self, _message: &[u8]) -> Result<Signature> {
        Err(alloy_signer::Error::other(
            "Browser signer requires async operations. Use sign_message instead.",
        ))
    }

    fn chain_id_sync(&self) -> Option<ChainId> {
        Some(self.chain_id)
    }
}

#[async_trait]
impl Signer for BrowserSigner {
    async fn sign_hash(&self, _hash: &B256) -> Result<Signature> {
        Err(alloy_signer::Error::other(
            "Browser wallets sign and send transactions in one step. Use eth_sendTransaction instead.",
        ))
    }

    async fn sign_typed_data<T: SolStruct + Send + Sync>(
        &self,
        _payload: &T,
        _domain: &Eip712Domain,
    ) -> Result<Signature>
    where
        Self: Sized,
    {
        // Not directly supported - use sign_dynamic_typed_data instead
        Err(alloy_signer::Error::other(
            "Browser wallets cannot sign typed data directly. Use sign_dynamic_typed_data instead.",
        ))
    }

    async fn sign_message(&self, message: &[u8]) -> Result<Signature> {
        let request = BrowserSignRequest {
            id: Uuid::new_v4(),
            sign_type: SignType::PersonalSign,
            request: SignRequest { message: hex::encode_prefixed(message), address: self.address },
        };

        let server = self.server.lock().await;
        let signature =
            server.request_signing(request).await.map_err(alloy_signer::Error::other)?;

        Signature::try_from(signature.as_ref())
            .map_err(|e| alloy_signer::Error::other(format!("Invalid signature: {e}")))
    }

    async fn sign_dynamic_typed_data(&self, payload: &TypedData) -> Result<Signature> {
        let server = self.server.lock().await;
        let signature = server
            .request_typed_data_signing(self.address, payload.clone())
            .await
            .map_err(alloy_signer::Error::other)?;

        // Parse the signature
        Signature::try_from(signature.as_ref())
            .map_err(|e| alloy_signer::Error::other(format!("Invalid signature: {e}")))
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
