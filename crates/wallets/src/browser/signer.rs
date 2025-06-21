use super::{
    communication::{SigningRequest, SigningType, TransactionRequest},
    server::BrowserWalletServer,
};
use alloy_consensus::SignableTransaction;
use alloy_network::TxSigner;
use alloy_primitives::{Address, ChainId, Signature, B256};
use alloy_signer::{Result, Signer};
use alloy_sol_types::{Eip712Domain, SolStruct};
use alloy_dyn_abi::TypedData;
use async_trait::async_trait;
use std::{str::FromStr, sync::Arc};
use tokio::sync::Mutex;

/// Browser wallet signer that delegates signing to a connected browser wallet.
///
/// This signer opens a local HTTP server and displays a web interface where users
/// can connect their browser wallet (MetaMask, WalletConnect browser extension, etc.)
/// to sign transactions.
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
        server.start().await.map_err(|e| alloy_signer::Error::other(e))?;
        
        // Wait for wallet connection
        println!("\n🌐 Opening browser for wallet connection...");
        println!("Waiting for wallet connection...\n");
        
        // Poll for connection (timeout after 5 minutes)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(300);
        
        loop {
            if let Some(connection) = server.get_connection() {
                println!("✅ Wallet connected: {}", connection.address);
                println!("   Chain ID: {}\n", connection.chain_id);
                
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
        to: Option<Address>,
        value: Option<String>,
        data: Option<String>,
        gas: Option<String>,
        gas_price: Option<String>,
        max_fee_per_gas: Option<String>,
        max_priority_fee_per_gas: Option<String>,
        nonce: Option<u64>,
        chain_id: Option<u64>,
    ) -> Result<B256> {
        let request = TransactionRequest {
            id: uuid::Uuid::new_v4().to_string(),
            from: format!("{:?}", self.address),
            to: to.map(|a| format!("{:?}", a)),
            value: value.unwrap_or_else(|| "0".to_string()),
            data,
            gas,
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            nonce,
            chain_id: chain_id.unwrap_or(self.chain_id),
        };
        
        let server = self.server.lock().await;
        let tx_hash = server
            .request_transaction(request)
            .await
            .map_err(|e| alloy_signer::Error::other(e))?;
        
        // Parse the transaction hash
        let hash = tx_hash
            .parse::<B256>()
            .map_err(|e| alloy_signer::Error::other(format!("Invalid transaction hash: {}", e)))?;
        
        // Give the UI a moment to update before potentially shutting down
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        Ok(hash)
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
        let request = SigningRequest {
            id: uuid::Uuid::new_v4().to_string(),
            from: format!("{:?}", self.address),
            signing_type: SigningType::PersonalSign,
            data: format!("0x{}", hex::encode(message)),
        };
        
        let server = self.server.lock().await;
        let signature = server
            .request_signing(request)
            .await
            .map_err(|e| alloy_signer::Error::other(e))?;
        
        // Parse the signature
        Signature::from_str(&signature).map_err(|e| alloy_signer::Error::other(format!("Invalid signature: {}", e)))
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
        Err(alloy_signer::Error::other(
            "Use sign_dynamic_typed_data for browser wallets"
        ))
    }
    
    async fn sign_dynamic_typed_data(&self, payload: &TypedData) -> Result<Signature> {
        // Serialize the typed data to JSON
        let json_data = serde_json::to_string(payload)
            .map_err(|e| alloy_signer::Error::other(format!("Failed to serialize typed data: {}", e)))?;
        
        let request = SigningRequest {
            id: uuid::Uuid::new_v4().to_string(),
            from: format!("{:?}", self.address),
            signing_type: SigningType::SignTypedData,
            data: json_data,
        };
        
        let server = self.server.lock().await;
        let signature = server
            .request_signing(request)
            .await
            .map_err(|e| alloy_signer::Error::other(e))?;
        
        // Parse the signature
        Signature::from_str(&signature).map_err(|e| alloy_signer::Error::other(format!("Invalid signature: {}", e)))
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
        Err(alloy_signer::Error::other(
            "Use send_transaction_via_browser for browser wallets"
        ))
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