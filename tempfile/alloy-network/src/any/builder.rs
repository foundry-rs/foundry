use crate::{
    any::AnyNetwork, BuildResult, Network, NetworkWallet, TransactionBuilder,
    TransactionBuilderError,
};
use alloy_primitives::{Address, Bytes, ChainId, TxKind, U256};
use alloy_rpc_types_eth::{AccessList, TransactionRequest};
use alloy_serde::WithOtherFields;
use std::ops::{Deref, DerefMut};

impl TransactionBuilder<AnyNetwork> for WithOtherFields<TransactionRequest> {
    fn chain_id(&self) -> Option<ChainId> {
        self.deref().chain_id()
    }

    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.deref_mut().set_chain_id(chain_id)
    }

    fn nonce(&self) -> Option<u64> {
        self.deref().nonce()
    }

    fn set_nonce(&mut self, nonce: u64) {
        self.deref_mut().set_nonce(nonce)
    }

    fn input(&self) -> Option<&Bytes> {
        self.deref().input()
    }

    fn set_input<T: Into<Bytes>>(&mut self, input: T) {
        self.deref_mut().set_input(input);
    }

    fn from(&self) -> Option<Address> {
        self.deref().from()
    }

    fn set_from(&mut self, from: Address) {
        self.deref_mut().set_from(from);
    }

    fn kind(&self) -> Option<TxKind> {
        self.deref().kind()
    }

    fn clear_kind(&mut self) {
        self.deref_mut().clear_kind()
    }

    fn set_kind(&mut self, kind: TxKind) {
        self.deref_mut().set_kind(kind)
    }

    fn value(&self) -> Option<U256> {
        self.deref().value()
    }

    fn set_value(&mut self, value: U256) {
        self.deref_mut().set_value(value)
    }

    fn gas_price(&self) -> Option<u128> {
        self.deref().gas_price()
    }

    fn set_gas_price(&mut self, gas_price: u128) {
        self.deref_mut().set_gas_price(gas_price);
    }

    fn max_fee_per_gas(&self) -> Option<u128> {
        self.deref().max_fee_per_gas()
    }

    fn set_max_fee_per_gas(&mut self, max_fee_per_gas: u128) {
        self.deref_mut().set_max_fee_per_gas(max_fee_per_gas);
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        self.deref().max_priority_fee_per_gas()
    }

    fn set_max_priority_fee_per_gas(&mut self, max_priority_fee_per_gas: u128) {
        self.deref_mut().set_max_priority_fee_per_gas(max_priority_fee_per_gas);
    }

    fn gas_limit(&self) -> Option<u64> {
        self.deref().gas_limit()
    }

    fn set_gas_limit(&mut self, gas_limit: u64) {
        self.deref_mut().set_gas_limit(gas_limit);
    }

    /// Get the EIP-2930 access list for the transaction.
    fn access_list(&self) -> Option<&AccessList> {
        self.deref().access_list()
    }

    /// Sets the EIP-2930 access list.
    fn set_access_list(&mut self, access_list: AccessList) {
        self.deref_mut().set_access_list(access_list)
    }

    fn complete_type(&self, ty: <AnyNetwork as Network>::TxType) -> Result<(), Vec<&'static str>> {
        self.deref().complete_type(ty.try_into().map_err(|_| vec!["supported tx type"])?)
    }

    fn can_submit(&self) -> bool {
        self.deref().can_submit()
    }

    fn can_build(&self) -> bool {
        self.deref().can_build()
    }

    #[doc(alias = "output_transaction_type")]
    fn output_tx_type(&self) -> <AnyNetwork as Network>::TxType {
        self.deref().output_tx_type().into()
    }

    #[doc(alias = "output_transaction_type_checked")]
    fn output_tx_type_checked(&self) -> Option<<AnyNetwork as Network>::TxType> {
        self.deref().output_tx_type_checked().map(Into::into)
    }

    fn prep_for_submission(&mut self) {
        self.deref_mut().prep_for_submission()
    }

    fn build_unsigned(self) -> BuildResult<<AnyNetwork as Network>::UnsignedTx, AnyNetwork> {
        if let Err((tx_type, missing)) = self.missing_keys() {
            return Err(TransactionBuilderError::InvalidTransactionRequest(
                tx_type.into(),
                missing,
            )
            .into_unbuilt(self));
        }
        Ok(self.inner.build_typed_tx().expect("checked by missing_keys").into())
    }

    async fn build<W: NetworkWallet<AnyNetwork>>(
        self,
        wallet: &W,
    ) -> Result<<AnyNetwork as Network>::TxEnvelope, TransactionBuilderError<AnyNetwork>> {
        Ok(wallet.sign_request(self).await?)
    }
}
