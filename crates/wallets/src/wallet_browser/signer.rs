use alloy_consensus::SignableTransaction;
use alloy_network::TxSigner;
use alloy_primitives::{Address, B256, ChainId, address};
use alloy_signer::{Result, Signature, Signer, SignerSync};
use async_trait::async_trait;

#[derive(Clone, Debug)]
pub struct BrowserSigner {
    address: Address,
    chain_id: ChainId,
}

impl BrowserSigner {
    pub async fn new(port: u16) -> Result<Self> {
        return Ok(Self {
            // TODO: Fetch address and chain ID from the browser wallet via the server.
            address: address!("0x0000000000000000000000000000000000000000"),
            chain_id: ChainId::default(),
        });
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
        // Browser wallets handle transaction signing differently
        // They sign and send in one step via eth_sendTransaction
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
        // Not used - browser wallets sign and send in one step
        Err(alloy_signer::Error::other("Use send_transaction_via_browser for browser wallets"))
    }
}
