use super::types::{DerivationType, TrezorError};
use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{
    hex, normalize_v, Address, ChainId, PrimitiveSignature as Signature, SignatureError, TxKind,
    B256, U256,
};
use alloy_signer::{sign_transaction_with_chain_id, Result, Signer};
use async_trait::async_trait;
use std::fmt;
use trezor_client::client::Trezor;

// we need firmware that supports EIP-1559 and EIP-712
const FIRMWARE_1_MIN_VERSION: &str = ">=1.11.1";
const FIRMWARE_2_MIN_VERSION: &str = ">=2.5.1";

/// A Trezor Ethereum signer.
///
/// This is a simple wrapper around the [Trezor transport](Trezor).
///
/// Note that this wallet only supports asynchronous operations. Calling a non-asynchronous method
/// will always return an error.
pub struct TrezorSigner {
    derivation: DerivationType,
    session_id: Vec<u8>,
    pub(crate) chain_id: Option<ChainId>,
    pub(crate) address: Address,
}

impl fmt::Debug for TrezorSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrezorSigner")
            .field("derivation", &self.derivation)
            .field("session_id", &hex::encode(&self.session_id))
            .field("address", &self.address)
            .finish()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Signer for TrezorSigner {
    #[inline]
    async fn sign_hash(&self, _hash: &B256) -> Result<Signature> {
        Err(alloy_signer::Error::UnsupportedOperation(
            alloy_signer::UnsupportedSignerOperation::SignHash,
        ))
    }

    #[inline]
    async fn sign_message(&self, message: &[u8]) -> Result<Signature> {
        self.sign_message_inner(message).await.map_err(alloy_signer::Error::other)
    }

    #[inline]
    fn address(&self) -> Address {
        self.address
    }

    #[inline]
    fn chain_id(&self) -> Option<ChainId> {
        self.chain_id
    }

    #[inline]
    fn set_chain_id(&mut self, chain_id: Option<ChainId>) {
        self.chain_id = chain_id;
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl alloy_network::TxSigner<Signature> for TrezorSigner {
    fn address(&self) -> Address {
        self.address
    }

    #[inline]
    #[doc(alias = "sign_tx")]
    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> Result<Signature> {
        sign_transaction_with_chain_id!(self, tx, self.sign_tx_inner(tx).await)
    }
}

impl TrezorSigner {
    /// Instantiates a new Trezor signer.
    #[instrument(ret)]
    pub async fn new(
        derivation: DerivationType,
        chain_id: Option<ChainId>,
    ) -> Result<Self, TrezorError> {
        let mut signer = Self {
            derivation: derivation.clone(),
            chain_id,
            address: Address::ZERO,
            session_id: vec![],
        };
        signer.initiate_session()?;
        signer.address = signer.get_address_with_path(&derivation).await?;
        Ok(signer)
    }

    fn check_version(version: semver::Version) -> Result<(), TrezorError> {
        let min_version = match version.major {
            1 => FIRMWARE_1_MIN_VERSION,
            2 => FIRMWARE_2_MIN_VERSION,
            // unknown major version, possibly newer models that we don't know about yet
            // it's probably safe to assume they support EIP-1559 and EIP-712
            _ => return Ok(()),
        };

        let req = semver::VersionReq::parse(min_version).unwrap();
        // Enforce firmware version is greater than "min_version"
        if !req.matches(&version) {
            return Err(TrezorError::UnsupportedFirmwareVersion(min_version.to_string()));
        }

        Ok(())
    }

    fn initiate_session(&mut self) -> Result<(), TrezorError> {
        let mut client = trezor_client::unique(false)?;
        client.init_device(None)?;

        let features = client.features().ok_or(TrezorError::Features)?;
        let version = semver::Version::new(
            features.major_version() as u64,
            features.minor_version() as u64,
            features.patch_version() as u64,
        );
        Self::check_version(version)?;

        self.session_id = features.session_id().to_vec();

        Ok(())
    }

    fn get_client(&self) -> Result<Trezor, TrezorError> {
        let mut client = trezor_client::unique(false)?;
        client.init_device(Some(self.session_id.clone()))?;
        Ok(client)
    }

    /// Get the account which corresponds to our derivation path
    pub async fn get_address(&self) -> Result<Address, TrezorError> {
        self.get_address_with_path(&self.derivation).await
    }

    /// Gets the account which corresponds to the provided derivation path
    #[instrument(ret)]
    pub async fn get_address_with_path(
        &self,
        derivation: &DerivationType,
    ) -> Result<Address, TrezorError> {
        let mut client = self.get_client()?;
        let address_str = client.ethereum_get_address(Self::convert_path(derivation))?;
        Ok(address_str.parse()?)
    }

    /// Signs an Ethereum transaction (requires confirmation on the Trezor).
    ///
    /// Does not apply EIP-155.
    #[doc(alias = "sign_transaction_inner")]
    async fn sign_tx_inner(
        &self,
        tx: &dyn SignableTransaction<Signature>,
    ) -> Result<Signature, TrezorError> {
        let mut client = self.get_client()?;
        let path = Self::convert_path(&self.derivation);

        let nonce = tx.nonce();
        let nonce = u64_to_trezor(nonce);

        let gas_price = tx.max_fee_per_gas();
        let gas_price = u128_to_trezor(gas_price);

        let gas_limit = tx.gas_limit();
        let gas_limit = u64_to_trezor(gas_limit);

        let to = match tx.kind() {
            TxKind::Call(to) => address_to_trezor(&to),
            TxKind::Create => String::new(),
        };

        let value = tx.value();
        let value = u256_to_trezor(value);

        let data = tx.input().to_vec();
        let chain_id = tx.chain_id();

        // TODO: Uncomment once dyn trait upcasting is stable
        /*
        let signature = if let Some(tx) = (tx as &dyn std::any::Any).downcast_ref::<TxEip1559>() {
        */
        let signature = if let Some(tx) = tx.__downcast_ref::<TxEip1559>() {
            let max_gas_fee = tx.max_fee_per_gas;
            let max_gas_fee = u128_to_trezor(max_gas_fee);

            let max_priority_fee = tx.max_priority_fee_per_gas;
            let max_priority_fee = u128_to_trezor(max_priority_fee);

            let access_list = tx
                .access_list
                .0
                .iter()
                .map(|item| trezor_client::client::AccessListItem {
                    address: address_to_trezor(&item.address),
                    storage_keys: item.storage_keys.iter().map(|key| key.to_vec()).collect(),
                })
                .collect();

            client.ethereum_sign_eip1559_tx(
                path,
                nonce,
                gas_limit,
                to,
                value,
                data,
                chain_id,
                max_gas_fee,
                max_priority_fee,
                access_list,
            )
        } else {
            client.ethereum_sign_tx(path, nonce, gas_price, gas_limit, to, value, data, chain_id)
        }?;
        signature_from_trezor(signature)
    }

    #[instrument(skip(message), fields(message=hex::encode(message)), ret)]
    async fn sign_message_inner(&self, message: &[u8]) -> Result<Signature, TrezorError> {
        let mut client = self.get_client()?;
        let apath = Self::convert_path(&self.derivation);
        let signature = client.ethereum_sign_message(message.into(), apath)?;
        signature_from_trezor(signature)
    }

    // helper which converts a derivation path to [u32]
    fn convert_path(derivation: &DerivationType) -> Vec<u32> {
        let derivation = derivation.to_string();
        let elements = derivation.split('/').skip(1).collect::<Vec<_>>();

        let mut path = vec![];
        for derivation_index in elements {
            let hardened = derivation_index.contains('\'');
            let mut index = derivation_index.replace('\'', "").parse::<u32>().unwrap();
            if hardened {
                index |= 0x80000000;
            }
            path.push(index);
        }

        path
    }
}

fn u64_to_trezor(x: u64) -> Vec<u8> {
    let bytes = x.to_be_bytes();
    bytes[x.leading_zeros() as usize / 8..].to_vec()
}

fn u128_to_trezor(x: u128) -> Vec<u8> {
    let bytes = x.to_be_bytes();
    bytes[x.leading_zeros() as usize / 8..].to_vec()
}

fn u256_to_trezor(x: U256) -> Vec<u8> {
    let bytes = x.to_be_bytes::<32>();
    bytes[x.leading_zeros() / 8..].to_vec()
}

fn address_to_trezor(x: &Address) -> String {
    format!("{x:?}")
}

fn signature_from_trezor(x: trezor_client::client::Signature) -> Result<Signature, TrezorError> {
    let r = U256::from_be_bytes(x.r);
    let s = U256::from_be_bytes(x.s);
    let v =
        normalize_v(x.v).ok_or(TrezorError::SignatureError(SignatureError::InvalidParity(x.v)))?;
    Ok(Signature::new(r, s, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::{EthereumWallet, TransactionBuilder};
    use alloy_primitives::{address, b256};
    use alloy_rpc_types_eth::{AccessList, AccessListItem, TransactionRequest};

    #[tokio::test]
    #[ignore]
    // Replace this with your ETH addresses.
    async fn test_get_address() {
        // Instantiate it with the default trezor derivation path
        let trezor = TrezorSigner::new(DerivationType::TrezorLive(1), Some(1)).await.unwrap();
        assert_eq!(
            trezor.get_address().await.unwrap(),
            address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
        );
        assert_eq!(
            trezor.get_address_with_path(&DerivationType::TrezorLive(0)).await.unwrap(),
            address!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_sign_message() {
        let trezor = TrezorSigner::new(DerivationType::TrezorLive(0), Some(1)).await.unwrap();
        let message = "hello world";
        let sig = trezor.sign_message(message.as_bytes()).await.unwrap();
        let addr = trezor.get_address().await.unwrap();
        assert_eq!(sig.recover_address_from_msg(message).unwrap(), addr);
    }

    #[tokio::test]
    #[ignore]
    async fn test_sign_tx() {
        let trezor = TrezorSigner::new(DerivationType::TrezorLive(0), Some(1)).await.unwrap();

        // approve uni v2 router 0xff
        let data = hex::decode("095ea7b30000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff").unwrap();
        let _tx = TransactionRequest::default()
            .to(address!("2ed7afa17473e17ac59908f088b4371d28585476"))
            .with_gas_limit(1000000)
            .with_gas_price(400e9 as u128)
            .with_nonce(5)
            .with_input(data)
            .with_value(U256::from(100e18 as u128))
            .build(&EthereumWallet::new(trezor))
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_sign_big_data_tx() {
        let trezor = TrezorSigner::new(DerivationType::TrezorLive(0), Some(1)).await.unwrap();

        // invalid data
        let big_data = hex::decode("095ea7b30000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string()+ &"ff".repeat(1032*2) + "aa").unwrap();
        let _tx = TransactionRequest::default()
            .to(address!("2ed7afa17473e17ac59908f088b4371d28585476"))
            .with_gas_limit(1000000)
            .with_gas_price(400e9 as u128)
            .with_nonce(5)
            .with_input(big_data)
            .with_value(U256::from(100e18 as u128))
            .build(&EthereumWallet::new(trezor))
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_sign_empty_txes() {
        let trezor = TrezorSigner::new(DerivationType::TrezorLive(0), Some(1)).await.unwrap();
        TransactionRequest::default()
            .to(address!("2ed7afa17473e17ac59908f088b4371d28585476"))
            .with_gas_price(1)
            .build(&EthereumWallet::new(trezor))
            .await
            .unwrap();

        let data = hex::decode("095ea7b30000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff").unwrap();

        // Contract creation (empty `to`, with data) should show on the trezor device as:
        //  ` "0 Wei ETH
        //  ` new contract?"
        let trezor = TrezorSigner::new(DerivationType::TrezorLive(0), Some(1)).await.unwrap();
        {
            let _tx = TransactionRequest::default()
                .into_create()
                .with_input(data)
                .with_gas_price(1)
                .build(&EthereumWallet::new(trezor))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_sign_eip1559_tx() {
        let trezor = TrezorSigner::new(DerivationType::TrezorLive(0), Some(1)).await.unwrap();

        // approve uni v2 router 0xff
        let data = hex::decode("095ea7b30000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff").unwrap();

        let lst = AccessList(vec![
            AccessListItem {
                address: address!("8ba1f109551bd432803012645ac136ddd64dba72"),
                storage_keys: vec![
                    b256!("0000000000000000000000000000000000000000000000000000000000000000"),
                    b256!("0000000000000000000000000000000000000000000000000000000000000042"),
                ],
            },
            AccessListItem {
                address: address!("2ed7afa17473e17ac59908f088b4371d28585476"),
                storage_keys: vec![
                    b256!("0000000000000000000000000000000000000000000000000000000000000000"),
                    b256!("0000000000000000000000000000000000000000000000000000000000000042"),
                ],
            },
        ]);

        let _tx = TransactionRequest::default()
            .to(address!("2ed7afa17473e17ac59908f088b4371d28585476"))
            .with_gas_limit(1000000)
            .max_fee_per_gas(400e9 as u128)
            .max_priority_fee_per_gas(400e9 as u128)
            .with_nonce(5)
            .with_input(data)
            .with_access_list(lst)
            .with_value(U256::from(100e18 as u128))
            .build(&EthereumWallet::new(trezor))
            .await
            .unwrap();
    }
}
