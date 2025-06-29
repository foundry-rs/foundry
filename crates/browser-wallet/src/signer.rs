use crate::{BrowserTransaction, BrowserWalletServer, SignRequest};
use alloy_consensus::SignableTransaction;
use alloy_dyn_abi::TypedData;
use alloy_network::TxSigner;
use alloy_primitives::{Address, ChainId, Signature, B256};
use alloy_rpc_types::TransactionRequest;
use alloy_signer::{Result, Signer, SignerSync};
use alloy_sol_types::{Eip712Domain, SolStruct};
use async_trait::async_trait;
use foundry_common::sh_println;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Browser wallet signer that delegates signing to a connected browser wallet.
///
/// This signer opens a local HTTP server and displays a web interface where users
/// can connect their browser wallet (MetaMask, WalletConnect browser extension, etc.)
/// to sign transactions.
///
/// # Standards
/// - Follows EIP-1193 for Ethereum Provider JavaScript API
/// - Supports EIP-712 for typed data signing
#[derive(Clone, Debug)]
pub struct BrowserSigner {
    server: Arc<Mutex<BrowserWalletServer>>,
    address: Address,
    chain_id: ChainId,
}

impl BrowserSigner {
    /// Create a new browser signer.
    ///
    /// This will start an HTTP server on the specified port and open a browser window
    /// for wallet connection. The function will wait for the user to connect their wallet
    /// before returning.
    ///
    /// # Arguments
    /// * `port` - The port to run the HTTP server on (use 0 for automatic assignment)
    pub async fn new(port: u16) -> Result<Self> {
        let mut server = BrowserWalletServer::new(port);

        // Start the server
        server.start().await.map_err(alloy_signer::Error::other)?;

        // Wait for wallet connection
        let _ = sh_println!("\n🌐 Opening browser for wallet connection...");
        let _ = sh_println!("Waiting for wallet connection...\n");

        // Poll for connection (timeout after 5 minutes)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(300);

        loop {
            if let Some(connection) = server.get_connection() {
                let _ = sh_println!("✅ Wallet connected: {}", connection.address);
                let _ = sh_println!("   Chain ID: {}\n", connection.chain_id);

                return Ok(Self {
                    server: Arc::new(Mutex::new(server)),
                    address: connection.address,
                    chain_id: connection.chain_id,
                });
            }

            if start.elapsed() > timeout {
                return Err(alloy_signer::Error::other("Wallet connection timeout"));
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    /// Send a transaction through the browser wallet.
    ///
    /// This method is used by cast send when browser wallet is detected.
    pub async fn send_transaction_via_browser(
        &self,
        tx_request: TransactionRequest,
    ) -> Result<B256> {
        let request =
            BrowserTransaction { id: uuid::Uuid::new_v4().to_string(), request: tx_request };

        let server = self.server.lock().await;
        let tx_hash =
            server.request_transaction(request).await.map_err(alloy_signer::Error::other)?;

        // Give the UI a moment to update before potentially shutting down
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        Ok(tx_hash)
    }
}

// Implement SignerSync trait as required by Alloy patterns
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
        // Browser wallets handle transaction signing differently
        // They sign and send in one step via eth_sendTransaction
        Err(alloy_signer::Error::other(
            "Browser wallets sign and send transactions in one step. Use eth_sendTransaction instead."
        ))
    }

    async fn sign_message(&self, message: &[u8]) -> Result<Signature> {
        let request = SignRequest {
            id: uuid::Uuid::new_v4().to_string(),
            address: self.address,
            message: format!("0x{}", hex::encode(message)),
            sign_type: crate::SignType::PersonalSign,
        };

        let server = self.server.lock().await;
        let signature =
            server.request_signing(request).await.map_err(alloy_signer::Error::other)?;

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

    async fn sign_typed_data<T: SolStruct + Send + Sync>(
        &self,
        _payload: &T,
        _domain: &Eip712Domain,
    ) -> Result<Signature>
    where
        Self: Sized,
    {
        // Not directly supported - use sign_dynamic_typed_data instead
        Err(alloy_signer::Error::other("Use sign_dynamic_typed_data for browser wallets"))
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
        // Not used - browser wallets sign and send in one step
        Err(alloy_signer::Error::other("Use send_transaction_via_browser for browser wallets"))
    }
}

impl Drop for BrowserSigner {
    fn drop(&mut self) {
        // Stop the server when the signer is dropped
        let server = self.server.clone();
        tokio::spawn(async move {
            let mut server = server.lock().await;
            let _ = server.stop().await;
        });
    }
}
