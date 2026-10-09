use alloy_dyn_abi::TypedData;
use alloy_primitives::{Address, Signature, map::AddressMap};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use jsonrpsee::core::async_trait;
use reth_ethereum::rpc::eth::SignError;
use reth_rpc_eth_api::{SignableTxRequest, helpers::EthSigner};
use std::marker::PhantomData;

/// Signs for the dev accounts with their private keys.
#[derive(Debug)]
pub struct DevSigner<T, TxReq> {
    accounts: AddressMap<PrivateKeySigner>,
    addresses: Vec<Address>,
    marker: PhantomData<fn() -> (T, TxReq)>,
}

impl<T, TxReq> DevSigner<T, TxReq> {
    /// Creates a signer for the given accounts, in order.
    pub fn new(accounts: Vec<PrivateKeySigner>) -> Self {
        let addresses = accounts.iter().map(|account| account.address()).collect();
        let accounts = accounts.into_iter().map(|account| (account.address(), account)).collect();
        Self { accounts, addresses, marker: PhantomData }
    }

    fn signer(&self, address: &Address) -> Result<&PrivateKeySigner, SignError> {
        self.accounts.get(address).ok_or(SignError::NoAccount)
    }
}

impl<T, TxReq> Clone for DevSigner<T, TxReq> {
    fn clone(&self) -> Self {
        Self {
            accounts: self.accounts.clone(),
            addresses: self.addresses.clone(),
            marker: PhantomData,
        }
    }
}

#[async_trait]
impl<T, TxReq> EthSigner<T, TxReq> for DevSigner<T, TxReq>
where
    T: Send + Sync + 'static,
    TxReq: SignableTxRequest<T> + Send + Sync + 'static,
{
    fn accounts(&self) -> Vec<Address> {
        self.addresses.clone()
    }

    fn is_signer_for(&self, address: &Address) -> bool {
        self.accounts.contains_key(address)
    }

    async fn sign(&self, address: Address, message: &[u8]) -> Result<Signature, SignError> {
        self.signer(&address)?.sign_message_sync(message).map_err(|_| SignError::CouldNotSign)
    }

    async fn sign_transaction(&self, request: TxReq, address: &Address) -> Result<T, SignError> {
        let signer = self.signer(address)?.clone();
        request.try_build_and_sign(signer).await.map_err(|_| SignError::InvalidTransactionRequest)
    }

    fn sign_typed_data(
        &self,
        address: Address,
        payload: &TypedData,
    ) -> Result<Signature, SignError> {
        self.signer(&address)?
            .sign_dynamic_typed_data_sync(payload)
            .map_err(|_| SignError::InvalidTypedData)
    }
}
