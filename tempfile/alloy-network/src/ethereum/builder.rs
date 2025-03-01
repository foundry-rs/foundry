use crate::{
    BuildResult, Ethereum, Network, NetworkWallet, TransactionBuilder, TransactionBuilder7702,
    TransactionBuilderError,
};
use alloy_consensus::{TxType, TypedTransaction};
use alloy_primitives::{Address, Bytes, ChainId, TxKind, U256};
use alloy_rpc_types_eth::{request::TransactionRequest, AccessList};

impl TransactionBuilder<Ethereum> for TransactionRequest {
    fn chain_id(&self) -> Option<ChainId> {
        self.chain_id
    }

    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.chain_id = Some(chain_id);
    }

    fn nonce(&self) -> Option<u64> {
        self.nonce
    }

    fn set_nonce(&mut self, nonce: u64) {
        self.nonce = Some(nonce);
    }

    fn input(&self) -> Option<&Bytes> {
        self.input.input()
    }

    fn set_input<T: Into<Bytes>>(&mut self, input: T) {
        self.input.input = Some(input.into());
    }

    fn from(&self) -> Option<Address> {
        self.from
    }

    fn set_from(&mut self, from: Address) {
        self.from = Some(from);
    }

    fn kind(&self) -> Option<TxKind> {
        self.to
    }

    fn clear_kind(&mut self) {
        self.to = None;
    }

    fn set_kind(&mut self, kind: TxKind) {
        self.to = Some(kind);
    }

    fn value(&self) -> Option<U256> {
        self.value
    }

    fn set_value(&mut self, value: U256) {
        self.value = Some(value)
    }

    fn gas_price(&self) -> Option<u128> {
        self.gas_price
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        self.gas_price = Some(gas_price);
    }

    fn max_fee_per_gas(&self) -> Option<u128> {
        self.max_fee_per_gas
    }

    fn set_max_fee_per_gas(&mut self, max_fee_per_gas: u128) {
        self.max_fee_per_gas = Some(max_fee_per_gas);
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.max_priority_fee_per_gas
    }

    fn set_max_priority_fee_per_gas(&mut self, max_priority_fee_per_gas: u128) {
        self.max_priority_fee_per_gas = Some(max_priority_fee_per_gas);
    }

    fn gas_limit(&self) -> Option<u64> {
        self.gas
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas = Some(gas_limit);
    }

    fn access_list(&self) -> Option<&AccessList> {
        self.access_list.as_ref()
    }

    fn set_access_list(&mut self, access_list: AccessList) {
        self.access_list = Some(access_list);
    }

    fn complete_type(&self, ty: TxType) -> Result<(), Vec<&'static str>> {
        match ty {
            TxType::Legacy => self.complete_legacy(),
            TxType::Eip2930 => self.complete_2930(),
            TxType::Eip1559 => self.complete_1559(),
            TxType::Eip4844 => self.complete_4844(),
            TxType::Eip7702 => self.complete_7702(),
        }
    }

    fn can_submit(&self) -> bool {
        // value and data may be None. If they are, they will be set to default.
        // gas fields and nonce may be None, if they are, they will be populated
        // with default values by the RPC server
        self.from.is_some()
    }

    fn can_build(&self) -> bool {
        // value and data may be none. If they are, they will be set to default
        // values.

        // chain_id and from may be none.
        let common = self.gas.is_some() && self.nonce.is_some();

        let legacy = self.gas_price.is_some();
        let eip2930 = legacy && self.access_list().is_some();

        let eip1559 = self.max_fee_per_gas.is_some() && self.max_priority_fee_per_gas.is_some();

        let eip4844 = eip1559 && self.sidecar.is_some() && self.to.is_some();

        let eip7702 = eip1559 && self.authorization_list().is_some();
        common && (legacy || eip2930 || eip1559 || eip4844 || eip7702)
    }

    #[doc(alias = "output_transaction_type")]
    fn output_tx_type(&self) -> TxType {
        self.preferred_type()
    }

    #[doc(alias = "output_transaction_type_checked")]
    fn output_tx_type_checked(&self) -> Option<TxType> {
        self.buildable_type()
    }

    fn prep_for_submission(&mut self) {
        self.transaction_type = Some(self.preferred_type() as u8);
        self.trim_conflicting_keys();
        self.populate_blob_hashes();
    }

    fn build_unsigned(self) -> BuildResult<TypedTransaction, Ethereum> {
        if let Err((tx_type, missing)) = self.missing_keys() {
            return Err(TransactionBuilderError::InvalidTransactionRequest(tx_type, missing)
                .into_unbuilt(self));
        }
        Ok(self.build_typed_tx().expect("checked by missing_keys"))
    }

    async fn build<W: NetworkWallet<Ethereum>>(
        self,
        wallet: &W,
    ) -> Result<<Ethereum as Network>::TxEnvelope, TransactionBuilderError<Ethereum>> {
        Ok(wallet.sign_request(self).await?)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        TransactionBuilder, TransactionBuilder4844, TransactionBuilder7702, TransactionBuilderError,
    };
    use alloy_consensus::{BlobTransactionSidecar, TxEip1559, TxType, TypedTransaction};
    use alloy_eips::eip7702::Authorization;
    use alloy_primitives::{Address, PrimitiveSignature as Signature, U256};
    use alloy_rpc_types_eth::{AccessList, TransactionRequest};
    use std::str::FromStr;

    #[test]
    fn from_eip1559_to_tx_req() {
        let tx = TxEip1559 {
            chain_id: 1,
            nonce: 0,
            gas_limit: 21_000,
            to: Address::ZERO.into(),
            max_priority_fee_per_gas: 20e9 as u128,
            max_fee_per_gas: 20e9 as u128,
            ..Default::default()
        };
        let tx_req: TransactionRequest = tx.into();
        tx_req.build_unsigned().unwrap();
    }

    #[test]
    fn test_4844_when_sidecar() {
        let request = TransactionRequest::default()
            .with_nonce(1)
            .with_gas_limit(0)
            .with_max_fee_per_gas(0)
            .with_max_priority_fee_per_gas(0)
            .with_to(Address::ZERO)
            .with_blob_sidecar(BlobTransactionSidecar::default())
            .with_max_fee_per_blob_gas(0);

        let tx = request.clone().build_unsigned().unwrap();

        assert!(matches!(tx, TypedTransaction::Eip4844(_)));

        let tx = request.with_gas_price(0).build_unsigned().unwrap();

        assert!(matches!(tx, TypedTransaction::Eip4844(_)));
    }

    #[test]
    fn test_2930_when_access_list() {
        let request = TransactionRequest::default()
            .with_nonce(1)
            .with_gas_limit(0)
            .with_max_fee_per_gas(0)
            .with_max_priority_fee_per_gas(0)
            .with_to(Address::ZERO)
            .with_gas_price(0)
            .with_access_list(AccessList::default());

        let tx = request.build_unsigned().unwrap();

        assert!(matches!(tx, TypedTransaction::Eip2930(_)));
    }

    #[test]
    fn test_7702_when_authorization_list() {
        let request = TransactionRequest::default()
            .with_nonce(1)
            .with_gas_limit(0)
            .with_max_fee_per_gas(0)
            .with_max_priority_fee_per_gas(0)
            .with_to(Address::ZERO)
            .with_access_list(AccessList::default())
            .with_authorization_list(vec![(Authorization {
                chain_id: U256::from(1),
                address: Address::left_padding_from(&[1]),
                nonce: 1u64,
            })
            .into_signed(Signature::from_str("48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b").unwrap())],);

        let tx = request.build_unsigned().unwrap();

        assert!(matches!(tx, TypedTransaction::Eip7702(_)));
    }

    #[test]
    fn test_default_to_1559() {
        let request = TransactionRequest::default()
            .with_nonce(1)
            .with_gas_limit(0)
            .with_max_fee_per_gas(0)
            .with_max_priority_fee_per_gas(0)
            .with_to(Address::ZERO);

        let tx = request.clone().build_unsigned().unwrap();

        assert!(matches!(tx, TypedTransaction::Eip1559(_)));

        let request = request.with_gas_price(0);
        let tx = request.build_unsigned().unwrap();
        assert!(matches!(tx, TypedTransaction::Legacy(_)));
    }

    #[test]
    fn test_fail_when_sidecar_and_access_list() {
        let request = TransactionRequest::default()
            .with_blob_sidecar(BlobTransactionSidecar::default())
            .with_access_list(AccessList::default());

        let error = request.build_unsigned().unwrap_err();

        assert!(matches!(error.error, TransactionBuilderError::InvalidTransactionRequest(_, _)));
    }

    #[test]
    fn test_invalid_legacy_fields() {
        let request = TransactionRequest::default().with_gas_price(0);

        let error = request.build_unsigned().unwrap_err();

        let TransactionBuilderError::InvalidTransactionRequest(tx_type, errors) = error.error
        else {
            panic!("wrong variant")
        };

        assert_eq!(tx_type, TxType::Legacy);
        assert_eq!(errors.len(), 3);
        assert!(errors.contains(&"to"));
        assert!(errors.contains(&"nonce"));
        assert!(errors.contains(&"gas_limit"));
    }

    #[test]
    fn test_invalid_1559_fields() {
        let request = TransactionRequest::default();

        let error = request.build_unsigned().unwrap_err();

        let TransactionBuilderError::InvalidTransactionRequest(tx_type, errors) = error.error
        else {
            panic!("wrong variant")
        };

        assert_eq!(tx_type, TxType::Eip1559);
        assert_eq!(errors.len(), 5);
        assert!(errors.contains(&"to"));
        assert!(errors.contains(&"nonce"));
        assert!(errors.contains(&"gas_limit"));
        assert!(errors.contains(&"max_priority_fee_per_gas"));
        assert!(errors.contains(&"max_fee_per_gas"));
    }

    #[test]
    fn test_invalid_2930_fields() {
        let request = TransactionRequest::default()
            .with_access_list(AccessList::default())
            .with_gas_price(Default::default());

        let error = request.build_unsigned().unwrap_err();

        let TransactionBuilderError::InvalidTransactionRequest(tx_type, errors) = error.error
        else {
            panic!("wrong variant")
        };

        assert_eq!(tx_type, TxType::Eip2930);
        assert_eq!(errors.len(), 3);
        assert!(errors.contains(&"to"));
        assert!(errors.contains(&"nonce"));
        assert!(errors.contains(&"gas_limit"));
    }

    #[test]
    fn test_invalid_4844_fields() {
        let request =
            TransactionRequest::default().with_blob_sidecar(BlobTransactionSidecar::default());

        let error = request.build_unsigned().unwrap_err();

        let TransactionBuilderError::InvalidTransactionRequest(tx_type, errors) = error.error
        else {
            panic!("wrong variant")
        };

        assert_eq!(tx_type, TxType::Eip4844);
        assert_eq!(errors.len(), 7);
        assert!(errors.contains(&"to"));
        assert!(errors.contains(&"nonce"));
        assert!(errors.contains(&"gas_limit"));
        assert!(errors.contains(&"max_priority_fee_per_gas"));
        assert!(errors.contains(&"max_fee_per_gas"));
        assert!(errors.contains(&"to"));
        assert!(errors.contains(&"max_fee_per_blob_gas"));
    }

    #[test]
    fn test_invalid_7702_fields() {
        let request = TransactionRequest::default().with_authorization_list(vec![]);

        let error = request.build_unsigned().unwrap_err();

        let TransactionBuilderError::InvalidTransactionRequest(tx_type, errors) = error.error
        else {
            panic!("wrong variant")
        };

        assert_eq!(tx_type, TxType::Eip7702);
        assert_eq!(errors.len(), 5);
        assert!(errors.contains(&"to"));
        assert!(errors.contains(&"nonce"));
        assert!(errors.contains(&"gas_limit"));
        assert!(errors.contains(&"max_priority_fee_per_gas"));
        assert!(errors.contains(&"max_fee_per_gas"));
    }
}
