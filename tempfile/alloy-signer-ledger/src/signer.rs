//! Ledger Ethereum app wrapper.

use crate::types::{DerivationType, LedgerError, INS, P1, P1_FIRST, P2};
use alloy_consensus::SignableTransaction;
use alloy_primitives::{
    hex, normalize_v, Address, ChainId, PrimitiveSignature as Signature, SignatureError, B256,
};
use alloy_signer::{sign_transaction_with_chain_id, Result, Signer};
use async_trait::async_trait;
use coins_ledger::{
    common::{APDUCommand, APDUData},
    transports::{Ledger, LedgerAsync},
};
use futures_util::lock::Mutex;

#[cfg(feature = "eip712")]
use alloy_dyn_abi::TypedData;
#[cfg(feature = "eip712")]
use alloy_sol_types::{Eip712Domain, SolStruct};

/// A Ledger Ethereum signer.
///
/// This is a simple wrapper around the [Ledger transport](Ledger).
///
/// Note that this wallet only supports asynchronous operations. Calling a non-asynchronous method
/// will always return an error.
#[derive(Debug)]
pub struct LedgerSigner {
    transport: Mutex<Ledger>,
    derivation: DerivationType,
    pub(crate) chain_id: Option<ChainId>,
    pub(crate) address: Address,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl alloy_network::TxSigner<Signature> for LedgerSigner {
    fn address(&self) -> Address {
        self.address
    }

    #[inline]
    #[doc(alias = "sign_tx")]
    async fn sign_transaction(
        &self,
        tx: &mut dyn SignableTransaction<Signature>,
    ) -> Result<Signature> {
        let encoded = tx.encoded_for_signing();

        match encoded.as_slice() {
            // Ledger requires passing EIP712 data to a separate instruction
            #[cfg(feature = "eip712")]
            [0x19, 0x1, data @ ..] => {
                let domain_sep = data
                    .get(..32)
                    .ok_or_else(|| {
                        alloy_signer::Error::other(
                            "eip712 encoded data did not have a domain separator",
                        )
                    })
                    .map(B256::from_slice)?;

                let hash = data[32..]
                    .get(..32)
                    .ok_or_else(|| {
                        alloy_signer::Error::other("eip712 encoded data did not have hash struct")
                    })
                    .map(B256::from_slice)?;

                sign_transaction_with_chain_id!(
                    self,
                    tx,
                    self.sign_typed_data_with_separator(&hash, &domain_sep).await
                )
            }
            // Usual flow
            encoded => sign_transaction_with_chain_id!(self, tx, self.sign_tx_rlp(encoded).await),
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Signer for LedgerSigner {
    async fn sign_hash(&self, _hash: &B256) -> Result<Signature> {
        Err(alloy_signer::Error::UnsupportedOperation(
            alloy_signer::UnsupportedSignerOperation::SignHash,
        ))
    }

    #[inline]
    async fn sign_message(&self, message: &[u8]) -> Result<Signature> {
        let mut payload = Self::path_to_bytes(&self.derivation);
        payload.extend_from_slice(&(message.len() as u32).to_be_bytes());
        payload.extend_from_slice(message);

        self.sign_payload(INS::SIGN_PERSONAL_MESSAGE, &payload)
            .await
            .map_err(alloy_signer::Error::other)
    }

    #[cfg(feature = "eip712")]
    #[inline]
    async fn sign_typed_data<T: SolStruct + Send + Sync>(
        &self,
        payload: &T,
        domain: &Eip712Domain,
    ) -> Result<Signature> {
        self.sign_typed_data_(&payload.eip712_hash_struct(), domain)
            .await
            .map_err(alloy_signer::Error::other)
    }

    #[cfg(feature = "eip712")]
    #[inline]
    async fn sign_dynamic_typed_data(&self, payload: &TypedData) -> Result<Signature> {
        self.sign_typed_data_(&payload.hash_struct()?, &payload.domain)
            .await
            .map_err(alloy_signer::Error::other)
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

impl LedgerSigner {
    /// Instantiate the application by acquiring a lock on the ledger device.
    ///
    /// # Examples
    ///
    /// ```
    /// # async fn foo() -> Result<(), Box<dyn std::error::Error>> {
    /// use alloy_signer_ledger::{HDPath, LedgerSigner};
    ///
    /// let ledger = LedgerSigner::new(HDPath::LedgerLive(0), Some(1)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(
        derivation: DerivationType,
        chain_id: Option<ChainId>,
    ) -> Result<Self, LedgerError> {
        let transport = Ledger::init().await?;
        let address = Self::get_address_with_path_transport(&transport, &derivation).await?;
        debug!(%address, "Connected to Ledger");

        Ok(Self { transport: Mutex::new(transport), derivation, chain_id, address })
    }

    /// Get the account which corresponds to our derivation path
    pub async fn get_address(&self) -> Result<Address, LedgerError> {
        self.get_address_with_path(&self.derivation).await
    }

    /// Gets the account which corresponds to the provided derivation path
    pub async fn get_address_with_path(
        &self,
        derivation: &DerivationType,
    ) -> Result<Address, LedgerError> {
        let transport = self.transport.lock().await;
        Self::get_address_with_path_transport(&transport, derivation).await
    }

    #[instrument(skip(transport))]
    async fn get_address_with_path_transport(
        transport: &Ledger,
        derivation: &DerivationType,
    ) -> Result<Address, LedgerError> {
        let data = APDUData::new(&Self::path_to_bytes(derivation));

        let command = APDUCommand {
            cla: 0xe0,
            ins: INS::GET_PUBLIC_KEY as u8,
            p1: P1::NON_CONFIRM as u8,
            p2: P2::NO_CHAINCODE as u8,
            data,
            response_len: None,
        };

        debug!("Dispatching get_address request to ethereum app");
        let answer = transport.exchange(&command).await?;
        let result = answer.data().ok_or(LedgerError::UnexpectedNullResponse)?;

        let address = {
            // extract the address from the response
            let offset = 1 + result[0] as usize;
            let address_str = &result[offset + 1..offset + 1 + result[offset] as usize];
            let mut address = [0; 20];
            address.copy_from_slice(&hex::decode(address_str)?);
            address.into()
        };
        debug!(?address, "Received address from device");
        Ok(address)
    }

    /// Returns the semver of the Ethereum ledger app
    pub async fn version(&self) -> Result<semver::Version, LedgerError> {
        let transport = self.transport.lock().await;

        let command = APDUCommand {
            cla: 0xe0,
            ins: INS::GET_APP_CONFIGURATION as u8,
            p1: P1::NON_CONFIRM as u8,
            p2: P2::NO_CHAINCODE as u8,
            data: APDUData::new(&[]),
            response_len: None,
        };

        debug!("Dispatching get_version");
        let answer = transport.exchange(&command).await?;
        let data = answer.data().ok_or(LedgerError::UnexpectedNullResponse)?;
        let &[_flags, major, minor, patch] = data else {
            return Err(LedgerError::ShortResponse { got: data.len(), expected: 4 });
        };
        let version = semver::Version::new(major as u64, minor as u64, patch as u64);
        debug!(%version, "Retrieved version from device");
        Ok(version)
    }

    /// Signs an Ethereum transaction's RLP bytes (requires confirmation on the ledger).
    ///
    /// Note that this does not apply EIP-155.
    #[doc(alias = "sign_transaction_rlp")]
    pub async fn sign_tx_rlp(&self, tx_rlp: &[u8]) -> Result<Signature, LedgerError> {
        let mut payload = Self::path_to_bytes(&self.derivation);
        payload.extend_from_slice(tx_rlp);
        self.sign_payload(INS::SIGN, &payload).await
    }

    #[cfg(feature = "eip712")]
    async fn sign_typed_data_with_separator(
        &self,
        hash_struct: &B256,
        separator: &B256,
    ) -> Result<Signature, LedgerError> {
        // See comment for v1.6.0 requirement
        // https://github.com/LedgerHQ/app-ethereum/issues/105#issuecomment-765316999
        const EIP712_MIN_VERSION: &str = ">=1.6.0";
        let req = semver::VersionReq::parse(EIP712_MIN_VERSION).unwrap();
        let version = self.version().await?;

        // Enforce app version is greater than EIP712_MIN_VERSION
        if !req.matches(&version) {
            return Err(LedgerError::UnsupportedAppVersion(EIP712_MIN_VERSION));
        }

        let mut data = Self::path_to_bytes(&self.derivation);
        data.extend_from_slice(separator.as_slice());
        data.extend_from_slice(hash_struct.as_slice());

        self.sign_payload(INS::SIGN_ETH_EIP_712, &data).await
    }

    #[cfg(feature = "eip712")]
    async fn sign_typed_data_(
        &self,
        hash_struct: &B256,
        domain: &Eip712Domain,
    ) -> Result<Signature, LedgerError> {
        self.sign_typed_data_with_separator(hash_struct, &domain.separator()).await
    }

    /// Helper function for signing either transaction data, personal messages or EIP712 derived
    /// structs.
    #[instrument(err, skip_all, fields(command = %command, payload = hex::encode(payload)))]
    async fn sign_payload(&self, command: INS, payload: &[u8]) -> Result<Signature, LedgerError> {
        let transport = self.transport.lock().await;
        let mut command = APDUCommand {
            cla: 0xe0,
            ins: command as u8,
            p1: P1_FIRST,
            p2: P2::NO_CHAINCODE as u8,
            data: APDUData::new(&[]),
            response_len: None,
        };

        let mut answer = None;
        // workaround for https://github.com/LedgerHQ/app-ethereum/issues/409
        // TODO: remove in future version
        let chunk_size =
            (0..=255).rev().find(|i| payload.len() % i != 3).expect("true for any length");

        // Iterate in 255 byte chunks
        for chunk in payload.chunks(chunk_size) {
            command.data = APDUData::new(chunk);

            debug!(chunk = hex::encode(chunk), "Dispatching packet to device");
            let res = transport.exchange(&command).await;
            debug!(?res, "Received response from device");
            let ans = res?;
            let data = ans.data().ok_or(LedgerError::UnexpectedNullResponse)?;
            debug!(response = hex::encode(data), "Received response from device");
            answer = Some(ans);

            // We need more data
            command.p1 = P1::MORE as u8;
        }
        drop(transport);

        let answer = answer.unwrap();
        let data = answer.data().unwrap();
        if data.len() != 65 {
            return Err(LedgerError::ShortResponse { got: data.len(), expected: 65 });
        }

        let parity = normalize_v(data[0] as u64)
            .ok_or(LedgerError::SignatureError(SignatureError::InvalidParity(data[0] as u64)))?;
        let sig = Signature::from_bytes_and_parity(&data[1..], parity);
        debug!(?sig, "Received signature from device");
        Ok(sig)
    }

    // helper which converts a derivation path to bytes
    fn path_to_bytes(derivation: &DerivationType) -> Vec<u8> {
        let derivation = derivation.to_string();
        let elements = derivation.split('/').skip(1).collect::<Vec<_>>();
        let depth = elements.len();

        let mut bytes = vec![depth as u8];
        for derivation_index in elements {
            let hardened = derivation_index.contains('\'');
            let mut index = derivation_index.replace('\'', "").parse::<u32>().unwrap();
            if hardened {
                index |= 0x80000000;
            }

            bytes.extend(index.to_be_bytes());
        }

        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::TxSigner;
    use alloy_primitives::{address, bytes, U256};
    use alloy_rlp::Decodable;
    use serial_test::serial;
    use std::sync::OnceLock;

    const DTYPE: DerivationType = DerivationType::LedgerLive(0);

    fn my_address() -> Address {
        static ADDRESS: OnceLock<Address> = OnceLock::new();
        *ADDRESS.get_or_init(|| {
            let var = "LEDGER_ADDRESS";
            std::env::var(var).expect(var).parse().expect(var)
        })
    }

    async fn init_ledger() -> LedgerSigner {
        let _ = tracing_subscriber::fmt::try_init();
        match LedgerSigner::new(DTYPE, None).await {
            Ok(ledger) => ledger,
            Err(e) => panic!("{e:?}\n{e}"),
        }
    }

    #[tokio::test]
    #[serial]
    #[ignore]
    async fn test_get_address() {
        let ledger = init_ledger().await;
        assert_eq!(ledger.get_address().await.unwrap(), my_address());
        assert_eq!(ledger.get_address_with_path(&DTYPE).await.unwrap(), my_address());
    }

    #[tokio::test]
    #[serial]
    #[ignore]
    async fn test_version() {
        let ledger = init_ledger().await;
        let version = ledger.version().await.unwrap();
        eprintln!("{version}");
        assert!(version.major >= 1);
    }

    #[tokio::test]
    #[serial]
    #[ignore]
    async fn test_sign_tx_legacy() {
        // https://github.com/gakonst/ethers-rs/blob/90b87bd85be98caa8bb592b67f3f9acbc8a409cf/ethers-signers/src/ledger/app.rs#L321
        let mut tx = alloy_consensus::TxLegacy {
            nonce: 5,
            gas_price: 400e9 as u128,
            gas_limit: 1000000,
            to: address!("2ed7afa17473e17ac59908f088b4371d28585476").into(),
            // TODO: this fails for some reason with 6a80 APDU_CODE_BAD_KEY_HANDLE
            // approve uni v2 router 0xff
            // input: bytes!("095ea7b30000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
            input: bytes!("01020304"),
            value: U256::from(100e18 as u128),
            chain_id: Some(69420),
        };
        /*
        assert_eq!(tx.encoded_for_signing(), hex!("f87005855d21dba000830f4240942ed7afa17473e17ac59908f088b4371d2858547689056bc75e2d63100000b844095ea7b30000000000000000000000007a250d5630b4cf539739df2c5dacb4c659f2488dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"));
        */
        test_sign_tx_generic(&mut tx).await;
    }

    #[tokio::test]
    #[serial]
    #[ignore]
    async fn test_sign_tx_eip2930() {
        // From the Ledger Ethereum app example: https://github.com/LedgerHQ/app-ethereum/blob/2264f677568cbc1e3177f9eccb3c14a229ab3255/examples/signTx.py#L104-L106
        /*
        let tx_rlp = hex!("01f8e60380018402625a0094cccccccccccccccccccccccccccccccccccccccc830186a0a4693c61390000000000000000000000000000000000000000000000000000000000000002f85bf859940000000000000000000000000000000000000102f842a00000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000060a780a09b8adcd2a4abd34b42d56fcd90b949f74ca9696dfe2b427bc39aa280bbf1924ca029af4a471bb2953b4e7933ea95880648552a9345424a1ac760189655ceb1832a");
        */
        // Skip signature.
        let tx_rlp = hex!("01f8a30380018402625a0094cccccccccccccccccccccccccccccccccccccccc830186a0a4693c61390000000000000000000000000000000000000000000000000000000000000002f85bf859940000000000000000000000000000000000000102f842a00000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000060a7");
        let mut untyped_rlp = &tx_rlp[1..];
        let mut tx = alloy_consensus::TxEip2930::decode(&mut untyped_rlp).unwrap();
        assert_eq!(hex::encode(tx.encoded_for_signing()), hex::encode(tx_rlp));
        test_sign_tx_generic(&mut tx).await;
    }

    #[tokio::test]
    #[serial]
    #[ignore]
    async fn test_sign_tx_eip1559() {
        // From the Ledger Ethereum app example: https://github.com/LedgerHQ/app-ethereum/blob/2264f677568cbc1e3177f9eccb3c14a229ab3255/examples/signTx.py#L100-L102
        let tx_rlp = hex!("02ef0306843b9aca008504a817c80082520894b2bb2b958afa2e96dab3f3ce7162b87daea39017872386f26fc1000080c0");
        let mut untyped_rlp = &tx_rlp[1..];
        let mut tx = alloy_consensus::TxEip1559::decode(&mut untyped_rlp).unwrap();
        assert_eq!(hex::encode(tx.encoded_for_signing()), hex::encode(tx_rlp));
        test_sign_tx_generic(&mut tx).await;
    }

    async fn test_sign_tx_generic(tx: &mut dyn SignableTransaction<Signature>) {
        let sighash = tx.signature_hash();
        let ledger = init_ledger().await;
        let sig = match ledger.sign_transaction(tx).await {
            Ok(sig) => sig,
            Err(e) => panic!("Failed signing transaction: {e}"),
        };
        assert_eq!(sig.recover_address_from_prehash(&sighash).unwrap(), my_address());
    }

    #[tokio::test]
    #[serial]
    #[ignore]
    async fn test_sign_message() {
        let ledger = init_ledger().await;
        let message = "hello world";
        let sig = ledger.sign_message(message.as_bytes()).await.unwrap();
        let addr = ledger.get_address().await.unwrap();
        assert_eq!(addr, my_address());
        assert_eq!(sig.recover_address_from_msg(message.as_bytes()).unwrap(), my_address());
    }
}
