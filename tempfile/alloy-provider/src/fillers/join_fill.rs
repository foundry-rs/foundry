use crate::{
    fillers::{FillProvider, FillerControlFlow, TxFiller},
    provider::SendableTx,
    Provider, ProviderLayer,
};
use alloy_network::Network;
use alloy_transport::TransportResult;
use futures::try_join;

/// A layer that can fill in a [`TransactionRequest`] with additional information by joining two
/// [`TxFiller`]s.
///
/// This struct is itself a [`TxFiller`], and can be nested to compose any number of fill layers.
///
/// [`TransactionRequest`]: alloy_rpc_types_eth::TransactionRequest
#[derive(Clone, Copy, Debug, Default)]
pub struct JoinFill<L, R> {
    left: L,
    right: R,
}

impl<L, R> JoinFill<L, R> {
    /// Creates a new `JoinFill` with the given layers.
    pub const fn new(left: L, right: R) -> Self {
        Self { left, right }
    }

    /// Get a reference to the left filler.
    pub const fn left(&self) -> &L {
        &self.left
    }

    /// Get a reference to the right filler.
    pub const fn right(&self) -> &R {
        &self.right
    }

    /// Get a mutable reference to the left filler.
    ///
    /// NB: this function exists to enable the [`crate::WalletProvider`] impl.
    pub(crate) fn right_mut(&mut self) -> &mut R {
        &mut self.right
    }
}

impl<L, R> JoinFill<L, R> {
    /// Get a request for the left filler, if the left filler is ready.
    async fn prepare_left<P, N>(
        &self,
        provider: &P,
        tx: &N::TransactionRequest,
    ) -> TransportResult<Option<L::Fillable>>
    where
        P: Provider<N>,
        L: TxFiller<N>,
        N: Network,
    {
        if self.left.ready(tx) {
            self.left.prepare(provider, tx).await.map(Some)
        } else {
            Ok(None)
        }
    }

    /// Get a prepare for the right filler, if the right filler is ready.
    async fn prepare_right<P, N>(
        &self,
        provider: &P,
        tx: &N::TransactionRequest,
    ) -> TransportResult<Option<R::Fillable>>
    where
        P: Provider<N>,
        R: TxFiller<N>,
        N: Network,
    {
        if self.right.ready(tx) {
            self.right.prepare(provider, tx).await.map(Some)
        } else {
            Ok(None)
        }
    }
}

impl<L, R, N> TxFiller<N> for JoinFill<L, R>
where
    L: TxFiller<N>,
    R: TxFiller<N>,
    N: Network,
{
    type Fillable = (Option<L::Fillable>, Option<R::Fillable>);

    fn status(&self, tx: &N::TransactionRequest) -> FillerControlFlow {
        self.left.status(tx).absorb(self.right.status(tx))
    }

    fn fill_sync(&self, tx: &mut SendableTx<N>) {
        self.left.fill_sync(tx);
        self.right.fill_sync(tx);
    }

    async fn prepare<P>(
        &self,
        provider: &P,
        tx: &N::TransactionRequest,
    ) -> TransportResult<Self::Fillable>
    where
        P: Provider<N>,
    {
        try_join!(self.prepare_left(provider, tx), self.prepare_right(provider, tx))
    }

    async fn fill(
        &self,
        to_fill: Self::Fillable,
        mut tx: SendableTx<N>,
    ) -> TransportResult<SendableTx<N>> {
        if let Some(to_fill) = to_fill.0 {
            tx = self.left.fill(to_fill, tx).await?;
        };
        if let Some(to_fill) = to_fill.1 {
            tx = self.right.fill(to_fill, tx).await?;
        };
        Ok(tx)
    }
}

impl<L, R, P, N> ProviderLayer<P, N> for JoinFill<L, R>
where
    L: TxFiller<N>,
    R: TxFiller<N>,
    P: Provider<N>,
    N: Network,
{
    type Provider = FillProvider<Self, P, N>;
    fn layer(&self, inner: P) -> Self::Provider {
        FillProvider::new(inner, self.clone())
    }
}
