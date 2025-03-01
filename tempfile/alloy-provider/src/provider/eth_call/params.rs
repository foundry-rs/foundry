use alloy_eips::BlockId;
use alloy_network::Network;
use alloy_rpc_types_eth::{state::StateOverride, Bundle, StateContext, TransactionIndex};
use serde::ser::SerializeSeq;
use std::borrow::Cow;

/// The parameters for an `"eth_call"` RPC request.
#[derive(Clone, Debug)]
pub struct EthCallParams<'req, N: Network> {
    data: Cow<'req, N::TransactionRequest>,
    pub(crate) block: Option<BlockId>,
    pub(crate) overrides: Option<Cow<'req, StateOverride>>,
}

impl<'req, N> EthCallParams<'req, N>
where
    N: Network,
{
    /// Instantiates a new `EthCallParams` with the given data (transaction).
    pub const fn new(data: &'req N::TransactionRequest) -> Self {
        Self { data: Cow::Borrowed(data), block: None, overrides: None }
    }

    /// Sets the block to use for this call.
    pub const fn with_block(mut self, block: BlockId) -> Self {
        self.block = Some(block);
        self
    }

    /// Sets the state overrides for this call.
    pub fn with_overrides(mut self, overrides: &'req StateOverride) -> Self {
        self.overrides = Some(Cow::Borrowed(overrides));
        self
    }

    /// Returns a reference to the state overrides if set.
    pub fn overrides(&self) -> Option<&StateOverride> {
        self.overrides.as_deref()
    }

    /// Returns a reference to the transaction data.
    pub fn data(&self) -> &N::TransactionRequest {
        &self.data
    }

    /// Returns the block.
    pub const fn block(&self) -> Option<BlockId> {
        self.block
    }

    /// Clones the tx data and overrides into owned data.
    pub fn into_owned(self) -> EthCallParams<'static, N> {
        EthCallParams {
            data: Cow::Owned(self.data.into_owned()),
            block: self.block,
            overrides: self.overrides.map(|o| Cow::Owned(o.into_owned())),
        }
    }
}

impl<N: Network> serde::Serialize for EthCallParams<'_, N> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let len = if self.overrides().is_some() { 3 } else { 2 };

        let mut seq = serializer.serialize_seq(Some(len))?;
        seq.serialize_element(&self.data())?;

        if let Some(overrides) = self.overrides() {
            seq.serialize_element(&self.block().unwrap_or_default())?;
            seq.serialize_element(overrides)?;
        } else if let Some(block) = self.block() {
            seq.serialize_element(&block)?;
        }

        seq.end()
    }
}

/// The parameters for an `"eth_callMany"` RPC request.
#[derive(Clone, Debug)]
pub struct EthCallManyParams<'req> {
    bundles: Cow<'req, Vec<Bundle>>,
    context: Option<StateContext>,
    overrides: Option<Cow<'req, StateOverride>>,
}

impl<'req> EthCallManyParams<'req> {
    /// Instantiates a new `EthCallManyParams` with the given bundles.
    pub const fn new(bundles: &'req Vec<Bundle>) -> Self {
        Self { bundles: Cow::Borrowed(bundles), context: None, overrides: None }
    }

    /// Sets the block in the [`StateContext`] for this call.
    pub fn with_block(mut self, block: BlockId) -> Self {
        let mut context = self.context.unwrap_or_default();
        context.block_number = Some(block);
        self.context = Some(context);
        self
    }

    /// Sets the transaction index in the [`StateContext`] for this call.
    pub fn with_transaction_index(mut self, tx_index: TransactionIndex) -> Self {
        let mut context = self.context.unwrap_or_default();
        context.transaction_index = Some(tx_index);
        self.context = Some(context);
        self
    }

    /// Sets the state context for this call.
    pub const fn with_context(mut self, context: StateContext) -> Self {
        self.context = Some(context);
        self
    }

    /// Sets the state overrides for this call.
    pub fn with_overrides(mut self, overrides: &'req StateOverride) -> Self {
        self.overrides = Some(Cow::Borrowed(overrides));
        self
    }

    /// Returns a reference to the state context if set.
    pub const fn context(&self) -> Option<&StateContext> {
        self.context.as_ref()
    }

    /// Returns a reference to the bundles.
    pub fn bundles(&self) -> &[Bundle] {
        &self.bundles
    }

    /// Returns a mutable reference to the bundles.
    pub fn bundles_mut(&mut self) -> &mut Vec<Bundle> {
        Cow::to_mut(&mut self.bundles)
    }

    /// Returns a reference to the state overrides if set.
    pub fn overrides(&self) -> Option<&StateOverride> {
        self.overrides.as_deref()
    }

    /// Clones the bundles, context, and overrides into owned data.
    pub fn into_owned(self) -> EthCallManyParams<'static> {
        EthCallManyParams {
            bundles: Cow::Owned(self.bundles.into_owned()),
            context: self.context,
            overrides: self.overrides.map(|o| Cow::Owned(o.into_owned())),
        }
    }
}

impl serde::Serialize for EthCallManyParams<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let len = if self.overrides().is_some() { 3 } else { 2 };

        let mut seq = serializer.serialize_seq(Some(len))?;
        seq.serialize_element(&self.bundles())?;

        if let Some(context) = self.context() {
            seq.serialize_element(context)?;
        }

        if let Some(overrides) = self.overrides() {
            seq.serialize_element(overrides)?;
        }

        seq.end()
    }
}
