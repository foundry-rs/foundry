use super::signer::NetworkWallet;
use crate::Network;
use alloy_primitives::{Address, Bytes, ChainId, TxKind, U256};
use alloy_rpc_types_eth::AccessList;
use alloy_sol_types::SolCall;
use futures_utils_wasm::impl_future;

pub use alloy_network_primitives::{TransactionBuilder4844, TransactionBuilder7702};

/// Result type for transaction builders
pub type BuildResult<T, N> = Result<T, UnbuiltTransactionError<N>>;

/// An unbuilt transaction, along with some error.
#[derive(Debug, thiserror::Error)]
#[error("Failed to build transaction: {error}")]
pub struct UnbuiltTransactionError<N: Network> {
    /// The original request that failed to build.
    pub request: N::TransactionRequest,
    /// The error that occurred.
    #[source]
    pub error: TransactionBuilderError<N>,
}

/// Error type for transaction builders.
#[derive(Debug, thiserror::Error)]
pub enum TransactionBuilderError<N: Network> {
    /// Invalid transaction request
    #[error("{0} transaction can't be built due to missing keys: {1:?}")]
    InvalidTransactionRequest(N::TxType, Vec<&'static str>),

    /// Signer cannot produce signature type required for transaction.
    #[error("Signer cannot produce signature type required for transaction")]
    UnsupportedSignatureType,

    /// Signer error.
    #[error(transparent)]
    Signer(#[from] alloy_signer::Error),

    /// A custom error.
    #[error("{0}")]
    Custom(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl<N: Network> TransactionBuilderError<N> {
    /// Instantiate a custom error.
    pub fn custom<E>(e: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Custom(Box::new(e))
    }

    /// Convert the error into an unbuilt transaction error.
    pub const fn into_unbuilt(self, request: N::TransactionRequest) -> UnbuiltTransactionError<N> {
        UnbuiltTransactionError { request, error: self }
    }
}

/// A Transaction builder for a network.
///
/// Transaction builders are primarily used to construct typed transactions that can be signed with
/// [`TransactionBuilder::build`], or unsigned typed transactions with
/// [`TransactionBuilder::build_unsigned`].
///
/// Transaction builders should be able to construct all available transaction types on a given
/// network.
#[doc(alias = "TxBuilder")]
pub trait TransactionBuilder<N: Network>: Default + Sized + Send + Sync + 'static {
    /// Get the chain ID for the transaction.
    fn chain_id(&self) -> Option<ChainId>;

    /// Set the chain ID for the transaction.
    fn set_chain_id(&mut self, chain_id: ChainId);

    /// Builder-pattern method for setting the chain ID.
    fn with_chain_id(mut self, chain_id: ChainId) -> Self {
        self.set_chain_id(chain_id);
        self
    }

    /// Get the nonce for the transaction.
    fn nonce(&self) -> Option<u64>;

    /// Set the nonce for the transaction.
    fn set_nonce(&mut self, nonce: u64);

    /// Builder-pattern method for setting the nonce.
    fn with_nonce(mut self, nonce: u64) -> Self {
        self.set_nonce(nonce);
        self
    }

    /// Get the input data for the transaction.
    fn input(&self) -> Option<&Bytes>;

    /// Set the input data for the transaction.
    fn set_input<T: Into<Bytes>>(&mut self, input: T);

    /// Builder-pattern method for setting the input data.
    fn with_input<T: Into<Bytes>>(mut self, input: T) -> Self {
        self.set_input(input);
        self
    }

    /// Get the sender for the transaction.
    fn from(&self) -> Option<Address>;

    /// Set the sender for the transaction.
    fn set_from(&mut self, from: Address);

    /// Builder-pattern method for setting the sender.
    fn with_from(mut self, from: Address) -> Self {
        self.set_from(from);
        self
    }

    /// Get the kind of transaction.
    fn kind(&self) -> Option<TxKind>;

    /// Clear the kind of transaction.
    fn clear_kind(&mut self);

    /// Set the kind of transaction.
    fn set_kind(&mut self, kind: TxKind);

    /// Builder-pattern method for setting the kind of transaction.
    fn with_kind(mut self, kind: TxKind) -> Self {
        self.set_kind(kind);
        self
    }

    /// Get the recipient for the transaction.
    fn to(&self) -> Option<Address> {
        if let Some(TxKind::Call(addr)) = self.kind() {
            return Some(addr);
        }
        None
    }

    /// Set the recipient for the transaction.
    fn set_to(&mut self, to: Address) {
        self.set_kind(to.into());
    }

    /// Builder-pattern method for setting the recipient.
    fn with_to(mut self, to: Address) -> Self {
        self.set_to(to);
        self
    }

    /// Set the `to` field to a create call.
    fn set_create(&mut self) {
        self.set_kind(TxKind::Create);
    }

    /// Set the `to` field to a create call.
    fn into_create(mut self) -> Self {
        self.set_create();
        self
    }

    /// Deploy the code by making a create call with data. This will set the
    /// `to` field to [`TxKind::Create`].
    fn set_deploy_code<T: Into<Bytes>>(&mut self, code: T) {
        self.set_input(code.into());
        self.set_create()
    }

    /// Deploy the code by making a create call with data. This will set the
    /// `to` field to [`TxKind::Create`].
    fn with_deploy_code<T: Into<Bytes>>(mut self, code: T) -> Self {
        self.set_deploy_code(code);
        self
    }

    /// Set the data field to a contract call. This will clear the `to` field
    /// if it is set to [`TxKind::Create`].
    fn set_call<T: SolCall>(&mut self, t: &T) {
        self.set_input(t.abi_encode());
        if matches!(self.kind(), Some(TxKind::Create)) {
            self.clear_kind();
        }
    }

    /// Make a contract call with data.
    fn with_call<T: SolCall>(mut self, t: &T) -> Self {
        self.set_call(t);
        self
    }

    /// Calculates the address that will be created by the transaction, if any.
    ///
    /// Returns `None` if the transaction is not a contract creation (the `to` field is set), or if
    /// the `from` or `nonce` fields are not set.
    fn calculate_create_address(&self) -> Option<Address> {
        if !self.kind().is_some_and(|to| to.is_create()) {
            return None;
        }
        let from = self.from()?;
        let nonce = self.nonce()?;
        Some(from.create(nonce))
    }

    /// Get the value for the transaction.
    fn value(&self) -> Option<U256>;

    /// Set the value for the transaction.
    fn set_value(&mut self, value: U256);

    /// Builder-pattern method for setting the value.
    fn with_value(mut self, value: U256) -> Self {
        self.set_value(value);
        self
    }

    /// Get the legacy gas price for the transaction.
    fn gas_price(&self) -> Option<u128>;

    /// Set the legacy gas price for the transaction.
    fn set_gas_price(&mut self, gas_price: u128);

    /// Builder-pattern method for setting the legacy gas price.
    fn with_gas_price(mut self, gas_price: u128) -> Self {
        self.set_gas_price(gas_price);
        self
    }

    /// Get the max fee per gas for the transaction.
    fn max_fee_per_gas(&self) -> Option<u128>;

    /// Set the max fee per gas  for the transaction.
    fn set_max_fee_per_gas(&mut self, max_fee_per_gas: u128);

    /// Builder-pattern method for setting max fee per gas .
    fn with_max_fee_per_gas(mut self, max_fee_per_gas: u128) -> Self {
        self.set_max_fee_per_gas(max_fee_per_gas);
        self
    }

    /// Get the max priority fee per gas for the transaction.
    fn max_priority_fee_per_gas(&self) -> Option<u128>;

    /// Set the max priority fee per gas for the transaction.
    fn set_max_priority_fee_per_gas(&mut self, max_priority_fee_per_gas: u128);

    /// Builder-pattern method for setting max priority fee per gas.
    fn with_max_priority_fee_per_gas(mut self, max_priority_fee_per_gas: u128) -> Self {
        self.set_max_priority_fee_per_gas(max_priority_fee_per_gas);
        self
    }
    /// Get the gas limit for the transaction.
    fn gas_limit(&self) -> Option<u64>;

    /// Set the gas limit for the transaction.
    fn set_gas_limit(&mut self, gas_limit: u64);

    /// Builder-pattern method for setting the gas limit.
    fn with_gas_limit(mut self, gas_limit: u64) -> Self {
        self.set_gas_limit(gas_limit);
        self
    }

    /// Get the EIP-2930 access list for the transaction.
    fn access_list(&self) -> Option<&AccessList>;

    /// Sets the EIP-2930 access list.
    fn set_access_list(&mut self, access_list: AccessList);

    /// Builder-pattern method for setting the access list.
    fn with_access_list(mut self, access_list: AccessList) -> Self {
        self.set_access_list(access_list);
        self
    }

    /// Check if all necessary keys are present to build the specified type,
    /// returning a list of missing keys.
    fn complete_type(&self, ty: N::TxType) -> Result<(), Vec<&'static str>>;

    /// Check if all necessary keys are present to build the currently-preferred
    /// transaction type, returning a list of missing keys.
    fn complete_preferred(&self) -> Result<(), Vec<&'static str>> {
        self.complete_type(self.output_tx_type())
    }

    /// Assert that the builder prefers a certain transaction type. This does
    /// not indicate that the builder is ready to build. This function uses a
    /// `dbg_assert_eq!` to check the builder status, and will have no affect
    /// in release builds.
    fn assert_preferred(&self, ty: N::TxType) {
        debug_assert_eq!(self.output_tx_type(), ty);
    }

    /// Assert that the builder prefers a certain transaction type. This does
    /// not indicate that the builder is ready to build. This function uses a
    /// `dbg_assert_eq!` to check the builder status, and will have no affect
    /// in release builds.
    fn assert_preferred_chained(self, ty: N::TxType) -> Self {
        self.assert_preferred(ty);
        self
    }

    /// Apply a function to the builder, returning the modified builder.
    fn apply<F>(self, f: F) -> Self
    where
        F: FnOnce(Self) -> Self,
    {
        f(self)
    }

    /// True if the builder contains all necessary information to be submitted
    /// to the `eth_sendTransaction` endpoint.
    fn can_submit(&self) -> bool;

    /// True if the builder contains all necessary information to be built into
    /// a valid transaction.
    fn can_build(&self) -> bool;

    /// Returns the transaction type that this builder will attempt to build.
    /// This does not imply that the builder is ready to build.
    #[doc(alias = "output_transaction_type")]
    fn output_tx_type(&self) -> N::TxType;

    /// Returns the transaction type that this builder will build. `None` if
    /// the builder is not ready to build.
    #[doc(alias = "output_transaction_type_checked")]
    fn output_tx_type_checked(&self) -> Option<N::TxType>;

    /// Trim any conflicting keys and populate any computed fields (like blob
    /// hashes).
    ///
    /// This is useful for transaction requests that have multiple conflicting
    /// fields. While these may be buildable, they may not be submitted to the
    /// RPC. This method should be called before RPC submission, but is not
    /// necessary before building.
    fn prep_for_submission(&mut self);

    /// Build an unsigned, but typed, transaction.
    fn build_unsigned(self) -> BuildResult<N::UnsignedTx, N>;

    /// Build a signed transaction.
    fn build<W: NetworkWallet<N>>(
        self,
        wallet: &W,
    ) -> impl_future!(<Output = Result<N::TxEnvelope, TransactionBuilderError<N>>>);
}
