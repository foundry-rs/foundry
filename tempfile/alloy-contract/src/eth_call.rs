use std::{future::IntoFuture, marker::PhantomData};

use crate::{Error, Result};
use alloy_dyn_abi::{DynSolValue, FunctionExt};
use alloy_json_abi::Function;
use alloy_network::Network;
use alloy_primitives::{Address, Bytes};
use alloy_rpc_types_eth::{
    state::{AccountOverride, StateOverride},
    BlockId,
};
use alloy_sol_types::SolCall;

/// Raw coder.
const RAW_CODER: () = ();

#[allow(unnameable_types)]
mod private {
    pub trait Sealed {}
    impl Sealed for super::Function {}
    impl<C: super::SolCall> Sealed for super::PhantomData<C> {}
    impl Sealed for () {}
}

/// An [`alloy_provider::EthCall`] with an abi decoder.
#[must_use = "EthCall must be awaited to execute the call"]
#[derive(Clone, Debug)]
pub struct EthCall<'req, 'coder, D, N>
where
    N: Network,
    D: CallDecoder,
{
    inner: alloy_provider::EthCall<'req, N, Bytes>,

    decoder: &'coder D,
}

impl<'req, 'coder, D, N> EthCall<'req, 'coder, D, N>
where
    N: Network,
    D: CallDecoder,
{
    /// Create a new [`EthCall`].
    pub const fn new(inner: alloy_provider::EthCall<'req, N, Bytes>, decoder: &'coder D) -> Self {
        Self { inner, decoder }
    }
}

impl<'req, N> EthCall<'req, 'static, (), N>
where
    N: Network,
{
    /// Create a new [`EthCall`].
    pub const fn new_raw(inner: alloy_provider::EthCall<'req, N, Bytes>) -> Self {
        Self::new(inner, &RAW_CODER)
    }
}

impl<'req, D, N> EthCall<'req, '_, D, N>
where
    N: Network,
    D: CallDecoder,
{
    /// Swap the decoder for this call.
    pub fn with_decoder<'new_coder, E>(
        self,
        decoder: &'new_coder E,
    ) -> EthCall<'req, 'new_coder, E, N>
    where
        E: CallDecoder,
    {
        EthCall { inner: self.inner, decoder }
    }

    /// Set the state overrides for this call.
    pub fn overrides(mut self, overrides: &'req StateOverride) -> Self {
        self.inner = self.inner.overrides(overrides);
        self
    }

    /// Appends a single [AccountOverride] to the state override.
    ///
    /// Creates a new [`StateOverride`] if none has been set yet.
    pub fn account_override(
        mut self,
        address: Address,
        account_overrides: AccountOverride,
    ) -> Self {
        self.inner = self.inner.account_override(address, account_overrides);
        self
    }
    /// Extends the the given [AccountOverride] to the state override.
    ///
    /// Creates a new [`StateOverride`] if none has been set yet.
    pub fn account_overrides(
        mut self,
        overrides: impl IntoIterator<Item = (Address, AccountOverride)>,
    ) -> Self {
        self.inner = self.inner.account_overrides(overrides);
        self
    }

    /// Set the block to use for this call.
    pub fn block(mut self, block: BlockId) -> Self {
        self.inner = self.inner.block(block);
        self
    }
}

impl<'req, N> From<alloy_provider::EthCall<'req, N, Bytes>> for EthCall<'req, 'static, (), N>
where
    N: Network,
{
    fn from(inner: alloy_provider::EthCall<'req, N, Bytes>) -> Self {
        Self { inner, decoder: &RAW_CODER }
    }
}

impl<'req, 'coder, D, N> std::future::IntoFuture for EthCall<'req, 'coder, D, N>
where
    D: CallDecoder + Unpin,
    N: Network,
{
    type Output = Result<D::CallOutput>;

    type IntoFuture = EthCallFut<'req, 'coder, D, N>;

    fn into_future(self) -> Self::IntoFuture {
        EthCallFut { inner: self.inner.into_future(), decoder: self.decoder }
    }
}

/// Future for the [`EthCall`] type. This future wraps an RPC call with an abi
/// decoder.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[derive(Debug)]
#[allow(unnameable_types)]
pub struct EthCallFut<'req, 'coder, D, N>
where
    N: Network,
    D: CallDecoder,
{
    inner: <alloy_provider::EthCall<'req, N, Bytes> as IntoFuture>::IntoFuture,
    decoder: &'coder D,
}

impl<D, N> std::future::Future for EthCallFut<'_, '_, D, N>
where
    D: CallDecoder + Unpin,
    N: Network,
{
    type Output = Result<D::CallOutput>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        let pin = std::pin::pin!(&mut this.inner);
        match pin.poll(cx) {
            std::task::Poll::Ready(Ok(data)) => {
                std::task::Poll::Ready(this.decoder.abi_decode_output(data, false))
            }
            std::task::Poll::Ready(Err(e)) => std::task::Poll::Ready(Err(e.into())),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// A trait for decoding the output of a contract function.
///
/// This trait is sealed and cannot be implemented manually.
/// It is an implementation detail of [`CallBuilder`].
///
/// [`CallBuilder`]: crate::CallBuilder
pub trait CallDecoder: private::Sealed {
    // Not public API.

    /// The output type of the contract function.
    #[doc(hidden)]
    type CallOutput;

    /// Decodes the output of a contract function.
    #[doc(hidden)]
    fn abi_decode_output(&self, data: Bytes, validate: bool) -> Result<Self::CallOutput>;

    #[doc(hidden)]
    fn as_debug_field(&self) -> impl std::fmt::Debug;
}

impl CallDecoder for Function {
    type CallOutput = Vec<DynSolValue>;

    #[inline]
    fn abi_decode_output(&self, data: Bytes, validate: bool) -> Result<Self::CallOutput> {
        FunctionExt::abi_decode_output(self, &data, validate)
            .map_err(|e| Error::decode(&self.name, &data, e))
    }

    #[inline]
    fn as_debug_field(&self) -> impl std::fmt::Debug {
        self
    }
}

impl<C: SolCall> CallDecoder for PhantomData<C> {
    type CallOutput = C::Return;

    #[inline]
    fn abi_decode_output(&self, data: Bytes, validate: bool) -> Result<Self::CallOutput> {
        C::abi_decode_returns(&data, validate)
            .map_err(|e| Error::decode(C::SIGNATURE, &data, e.into()))
    }

    #[inline]
    fn as_debug_field(&self) -> impl std::fmt::Debug {
        std::any::type_name::<C>()
    }
}

impl CallDecoder for () {
    type CallOutput = Bytes;

    #[inline]
    fn abi_decode_output(&self, data: Bytes, _validate: bool) -> Result<Self::CallOutput> {
        Ok(data)
    }

    #[inline]
    fn as_debug_field(&self) -> impl std::fmt::Debug {
        format_args!("()")
    }
}
