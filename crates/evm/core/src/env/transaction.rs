//! Mutable transaction environments and execution-family adapters.

use alloy_primitives::{Address, B256, Bytes, U256};
use revm::{
    context::{Transaction, TxEnv},
    context_interface::{
        either::Either,
        transaction::{AccessList, RecoveredAuthorization, SignedAuthorization},
    },
    primitives::TxKind,
};
use tempo_revm::TempoTxEnv;

#[cfg(feature = "optimism")]
use op_revm::transaction::deposit::DEPOSIT_TRANSACTION_TYPE;

/// Extension of [`Transaction`] with mutable setters, allowing EVM-agnostic mutation of transaction
/// fields.
pub trait FoundryTransaction: Transaction {
    /// Sets the transaction type.
    fn set_tx_type(&mut self, tx_type: u8);

    /// Sets the caller (sender) address.
    fn set_caller(&mut self, caller: Address);

    /// Sets the gas limit.
    fn set_gas_limit(&mut self, gas_limit: u64);

    /// Sets the gas price (or max fee per gas for EIP-1559).
    fn set_gas_price(&mut self, gas_price: u128);

    /// Sets the transaction kind (call or create).
    fn set_kind(&mut self, kind: TxKind);

    /// Sets the value sent with the transaction.
    fn set_value(&mut self, value: U256);

    /// Sets the transaction input data.
    fn set_data(&mut self, data: Bytes);

    /// Sets the nonce.
    fn set_nonce(&mut self, nonce: u64);

    /// Sets the chain ID.
    fn set_chain_id(&mut self, chain_id: Option<u64>);

    /// Sets the access list.
    fn set_access_list(&mut self, access_list: AccessList);

    /// Returns a mutable reference to the EIP-7702 authorization list.
    fn authorization_list_mut(
        &mut self,
    ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>>;

    /// Sets the max priority fee per gas.
    fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>);

    /// Sets the blob versioned hashes.
    fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>);

    /// Sets the max fee per blob gas.
    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128);

    /// Sets the EIP-7702 signed authorization list.
    fn set_signed_authorization(&mut self, auth: Vec<SignedAuthorization>) {
        *self.authorization_list_mut() = auth.into_iter().map(Either::Left).collect();
    }

    // `OpTransaction` methods

    /// Enveloped transaction bytes.
    fn enveloped_tx(&self) -> Option<&Bytes> {
        None
    }

    /// Set Enveloped transaction bytes.
    fn set_enveloped_tx(&mut self, _bytes: Bytes) {}

    /// Source hash of the deposit transaction.
    fn source_hash(&self) -> Option<B256> {
        None
    }

    /// Sets source hash of the deposit transaction.
    fn set_source_hash(&mut self, _source_hash: B256) {}

    /// Mint of the deposit transaction
    fn mint(&self) -> Option<u128> {
        None
    }

    /// Sets mint of the deposit transaction.
    fn set_mint(&mut self, _mint: u128) {}

    /// Whether the transaction is a system transaction
    fn is_system_transaction(&self) -> bool {
        false
    }

    /// Sets whether the transaction is a system transaction
    fn set_system_transaction(&mut self, _is_system_transaction: bool) {}

    /// Returns `true` if transaction is an Optimism deposit transaction.
    fn is_deposit(&self) -> bool {
        #[cfg(feature = "optimism")]
        {
            self.tx_type() == DEPOSIT_TRANSACTION_TYPE
        }
        #[cfg(not(feature = "optimism"))]
        {
            false
        }
    }

    // Tempo methods

    /// Returns the fee token address for this transaction.
    fn fee_token(&self) -> Option<Address> {
        None
    }

    /// Sets the fee token address for this transaction.
    fn set_fee_token(&mut self, _token: Option<Address>) {}

    /// Returns the fee payer for this transaction.
    fn fee_payer(&self) -> Option<Option<Address>> {
        None
    }

    /// Sets the fee payer for this transaction.
    fn set_fee_payer(&mut self, _payer: Option<Option<Address>>) {}
}

impl FoundryTransaction for TxEnv {
    fn set_tx_type(&mut self, tx_type: u8) {
        self.tx_type = tx_type;
    }

    fn set_caller(&mut self, caller: Address) {
        self.caller = caller;
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.gas_limit = gas_limit;
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        self.gas_price = gas_price;
    }

    fn set_kind(&mut self, kind: TxKind) {
        self.kind = kind;
    }

    fn set_value(&mut self, value: U256) {
        self.value = value;
    }

    fn set_data(&mut self, data: Bytes) {
        self.data = data;
    }

    fn set_nonce(&mut self, nonce: u64) {
        self.nonce = nonce;
    }

    fn set_chain_id(&mut self, chain_id: Option<u64>) {
        self.chain_id = chain_id;
    }

    fn set_access_list(&mut self, access_list: AccessList) {
        self.access_list = access_list;
    }

    fn authorization_list_mut(
        &mut self,
    ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
        &mut self.authorization_list
    }

    fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
        self.gas_priority_fee = gas_priority_fee;
    }

    fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>) {
        self.blob_hashes = blob_hashes;
    }

    fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
        self.max_fee_per_blob_gas = max_fee_per_blob_gas;
    }
}

impl FoundryTransaction for TempoTxEnv {
    fn set_tx_type(&mut self, tx_type: u8) {
        self.inner.set_tx_type(tx_type);
    }

    fn set_caller(&mut self, caller: Address) {
        self.inner.set_caller(caller);
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.inner.set_gas_limit(gas_limit);
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        self.inner.set_gas_price(gas_price);
    }

    fn set_kind(&mut self, kind: TxKind) {
        self.inner.set_kind(kind);
        if let Some(call) =
            self.tempo_tx_env.as_deref_mut().and_then(|env| env.aa_calls.first_mut())
        {
            call.to = kind;
        }
    }

    fn set_value(&mut self, value: U256) {
        self.inner.set_value(value);
        if let Some(call) =
            self.tempo_tx_env.as_deref_mut().and_then(|env| env.aa_calls.first_mut())
        {
            call.value = value;
        }
    }

    fn set_data(&mut self, data: Bytes) {
        self.inner.set_data(data.clone());
        if let Some(call) =
            self.tempo_tx_env.as_deref_mut().and_then(|env| env.aa_calls.first_mut())
        {
            call.input = data;
        }
    }

    fn set_nonce(&mut self, nonce: u64) {
        self.inner.set_nonce(nonce);
    }

    fn set_chain_id(&mut self, chain_id: Option<u64>) {
        self.inner.set_chain_id(chain_id);
    }

    fn set_access_list(&mut self, access_list: AccessList) {
        self.inner.set_access_list(access_list);
    }

    fn authorization_list_mut(
        &mut self,
    ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
        self.inner.authorization_list_mut()
    }

    fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
        self.inner.set_gas_priority_fee(gas_priority_fee);
    }

    fn set_blob_hashes(&mut self, _blob_hashes: Vec<B256>) {}

    fn set_max_fee_per_blob_gas(&mut self, _max_fee_per_blob_gas: u128) {}

    fn fee_token(&self) -> Option<Address> {
        self.fee_token
    }

    fn set_fee_token(&mut self, token: Option<Address>) {
        self.fee_token = token;
    }

    fn fee_payer(&self) -> Option<Option<Address>> {
        self.fee_payer
    }

    fn set_fee_payer(&mut self, payer: Option<Option<Address>>) {
        self.fee_payer = payer;
    }
}

#[cfg(feature = "base")]
mod base {
    use super::*;
    use base_common_evm::{BaseTransaction, BaseTxTr, DEPOSIT_TRANSACTION_TYPE};

    impl<TX: FoundryTransaction> FoundryTransaction for BaseTransaction<TX> {
        fn set_tx_type(&mut self, tx_type: u8) {
            self.base.set_tx_type(tx_type);
        }

        fn set_caller(&mut self, caller: Address) {
            self.base.set_caller(caller);
        }

        fn set_gas_limit(&mut self, gas_limit: u64) {
            self.base.set_gas_limit(gas_limit);
        }

        fn set_gas_price(&mut self, gas_price: u128) {
            self.base.set_gas_price(gas_price);
        }

        fn set_kind(&mut self, kind: TxKind) {
            self.base.set_kind(kind);
        }

        fn set_value(&mut self, value: U256) {
            self.base.set_value(value);
        }

        fn set_data(&mut self, data: Bytes) {
            self.base.set_data(data);
        }

        fn set_nonce(&mut self, nonce: u64) {
            self.base.set_nonce(nonce);
        }

        fn set_chain_id(&mut self, chain_id: Option<u64>) {
            self.base.set_chain_id(chain_id);
        }

        fn set_access_list(&mut self, access_list: AccessList) {
            self.base.set_access_list(access_list);
        }

        fn authorization_list_mut(
            &mut self,
        ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
            self.base.authorization_list_mut()
        }

        fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
            self.base.set_gas_priority_fee(gas_priority_fee);
        }

        fn set_blob_hashes(&mut self, blob_hashes: Vec<B256>) {
            self.base.set_blob_hashes(blob_hashes);
        }

        fn set_max_fee_per_blob_gas(&mut self, max_fee_per_blob_gas: u128) {
            self.base.set_max_fee_per_blob_gas(max_fee_per_blob_gas);
        }

        fn enveloped_tx(&self) -> Option<&Bytes> {
            BaseTxTr::enveloped_tx(self)
        }

        fn set_enveloped_tx(&mut self, bytes: Bytes) {
            self.enveloped_tx = Some(bytes);
        }

        fn source_hash(&self) -> Option<B256> {
            BaseTxTr::source_hash(self)
        }

        fn set_source_hash(&mut self, source_hash: B256) {
            self.deposit.source_hash = source_hash;
        }

        fn mint(&self) -> Option<u128> {
            BaseTxTr::mint(self)
        }

        fn set_mint(&mut self, mint: u128) {
            self.deposit.mint = Some(mint);
        }

        fn is_system_transaction(&self) -> bool {
            BaseTxTr::is_system_transaction(self)
        }

        fn set_system_transaction(&mut self, is_system_transaction: bool) {
            self.deposit.is_system_transaction = is_system_transaction;
        }

        fn is_deposit(&self) -> bool {
            self.tx_type() == DEPOSIT_TRANSACTION_TYPE
        }
    }
}

#[cfg(feature = "optimism")]
mod optimism {
    use super::*;
    use alloy_op_evm::OpTx;
    use op_revm::{OpTransaction, transaction::OpTxTr};

    impl<TX: FoundryTransaction> FoundryTransaction for OpTransaction<TX> {
        fn set_tx_type(&mut self, tx_type: u8) {
            self.base.set_tx_type(tx_type);
        }

        fn set_caller(&mut self, caller: Address) {
            self.base.set_caller(caller);
        }

        fn set_gas_limit(&mut self, gas_limit: u64) {
            self.base.set_gas_limit(gas_limit);
        }

        fn set_gas_price(&mut self, gas_price: u128) {
            self.base.set_gas_price(gas_price);
        }

        fn set_kind(&mut self, kind: TxKind) {
            self.base.set_kind(kind);
        }

        fn set_value(&mut self, value: U256) {
            self.base.set_value(value);
        }

        fn set_data(&mut self, data: Bytes) {
            self.base.set_data(data);
        }

        fn set_nonce(&mut self, nonce: u64) {
            self.base.set_nonce(nonce);
        }

        fn set_chain_id(&mut self, chain_id: Option<u64>) {
            self.base.set_chain_id(chain_id);
        }

        fn set_access_list(&mut self, access_list: AccessList) {
            self.base.set_access_list(access_list);
        }

        fn authorization_list_mut(
            &mut self,
        ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
            self.base.authorization_list_mut()
        }

        fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
            self.base.set_gas_priority_fee(gas_priority_fee);
        }

        fn set_blob_hashes(&mut self, _blob_hashes: Vec<B256>) {}

        fn set_max_fee_per_blob_gas(&mut self, _max_fee_per_blob_gas: u128) {}

        fn enveloped_tx(&self) -> Option<&Bytes> {
            OpTxTr::enveloped_tx(self)
        }

        fn set_enveloped_tx(&mut self, bytes: Bytes) {
            self.enveloped_tx = Some(bytes);
        }

        fn source_hash(&self) -> Option<B256> {
            OpTxTr::source_hash(self)
        }

        fn set_source_hash(&mut self, source_hash: B256) {
            if self.tx_type() == DEPOSIT_TRANSACTION_TYPE {
                self.deposit.source_hash = source_hash;
            }
        }

        fn mint(&self) -> Option<u128> {
            OpTxTr::mint(self)
        }

        fn set_mint(&mut self, mint: u128) {
            if self.tx_type() == DEPOSIT_TRANSACTION_TYPE {
                self.deposit.mint = Some(mint);
            }
        }

        fn is_system_transaction(&self) -> bool {
            OpTxTr::is_system_transaction(self)
        }

        fn set_system_transaction(&mut self, is_system_transaction: bool) {
            if self.tx_type() == DEPOSIT_TRANSACTION_TYPE {
                self.deposit.is_system_transaction = is_system_transaction;
            }
        }
    }

    impl FoundryTransaction for OpTx {
        fn set_tx_type(&mut self, tx_type: u8) {
            self.0.set_tx_type(tx_type);
        }

        fn set_caller(&mut self, caller: Address) {
            self.0.set_caller(caller);
        }

        fn set_gas_limit(&mut self, gas_limit: u64) {
            self.0.set_gas_limit(gas_limit);
        }

        fn set_gas_price(&mut self, gas_price: u128) {
            self.0.set_gas_price(gas_price);
        }

        fn set_kind(&mut self, kind: TxKind) {
            self.0.set_kind(kind);
        }

        fn set_value(&mut self, value: U256) {
            self.0.set_value(value);
        }

        fn set_data(&mut self, data: Bytes) {
            self.0.set_data(data);
        }

        fn set_nonce(&mut self, nonce: u64) {
            self.0.set_nonce(nonce);
        }

        fn set_chain_id(&mut self, chain_id: Option<u64>) {
            self.0.set_chain_id(chain_id);
        }

        fn set_access_list(&mut self, access_list: AccessList) {
            self.0.set_access_list(access_list);
        }

        fn authorization_list_mut(
            &mut self,
        ) -> &mut Vec<Either<SignedAuthorization, RecoveredAuthorization>> {
            self.0.authorization_list_mut()
        }

        fn set_gas_priority_fee(&mut self, gas_priority_fee: Option<u128>) {
            self.0.set_gas_priority_fee(gas_priority_fee);
        }

        fn set_blob_hashes(&mut self, _blob_hashes: Vec<B256>) {}

        fn set_max_fee_per_blob_gas(&mut self, _max_fee_per_blob_gas: u128) {}

        fn enveloped_tx(&self) -> Option<&Bytes> {
            FoundryTransaction::enveloped_tx(&self.0)
        }

        fn set_enveloped_tx(&mut self, bytes: Bytes) {
            self.0.set_enveloped_tx(bytes);
        }

        fn source_hash(&self) -> Option<B256> {
            FoundryTransaction::source_hash(&self.0)
        }

        fn set_source_hash(&mut self, source_hash: B256) {
            self.0.set_source_hash(source_hash);
        }

        fn mint(&self) -> Option<u128> {
            FoundryTransaction::mint(&self.0)
        }

        fn set_mint(&mut self, mint: u128) {
            self.0.set_mint(mint);
        }

        fn is_system_transaction(&self) -> bool {
            FoundryTransaction::is_system_transaction(&self.0)
        }

        fn set_system_transaction(&mut self, is_system_transaction: bool) {
            self.0.set_system_transaction(is_system_transaction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_alloy::primitives::transaction::Call;

    #[test]
    fn tempo_tx_env_setters_update_aa_call_payload() {
        let old_to = TxKind::Call(Address::with_last_byte(0xAA));
        let new_to = TxKind::Create;
        let new_value = U256::from(123);
        let new_input = Bytes::from_static(b"local bytecode");

        let mut tx_env = TempoTxEnv {
            inner: TxEnv {
                kind: old_to,
                value: U256::ONE,
                data: Bytes::from_static(b"original bytecode"),
                ..Default::default()
            },
            tempo_tx_env: Some(Box::new(tempo_revm::TempoBatchCallEnv {
                aa_calls: vec![Call {
                    to: old_to,
                    value: U256::ONE,
                    input: Bytes::from_static(b"original bytecode"),
                }],
                ..Default::default()
            })),
            ..Default::default()
        };

        tx_env.set_kind(new_to);
        tx_env.set_value(new_value);
        tx_env.set_data(new_input.clone());

        assert_eq!(tx_env.inner.kind, new_to);
        assert_eq!(tx_env.inner.value, new_value);
        assert_eq!(tx_env.inner.data, new_input);

        let call = &tx_env.tempo_tx_env.as_ref().unwrap().aa_calls[0];
        assert_eq!(call.to, new_to);
        assert_eq!(call.value, new_value);
        assert_eq!(call.input, new_input);
    }
}
