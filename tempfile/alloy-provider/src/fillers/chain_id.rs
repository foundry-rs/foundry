use std::sync::{Arc, OnceLock};

use alloy_network::{Network, TransactionBuilder};
use alloy_primitives::ChainId;
use alloy_transport::TransportResult;

use crate::{
    fillers::{FillerControlFlow, TxFiller},
    provider::SendableTx,
};

/// A [`TxFiller`] that populates the chain ID of a transaction.
///
/// If a chain ID is provided, it will be used for filling. If a chain ID
/// is not provided, the filler will attempt to fetch the chain ID from the
/// provider the first time a transaction is prepared, and will cache it for
/// future transactions.
///
/// Transactions that already have a chain_id set by the user will not be
/// modified.
///
/// # Example
///
/// ```
/// # use alloy_network::{NetworkWallet, EthereumWallet, Ethereum};
/// # use alloy_rpc_types_eth::TransactionRequest;
/// # use alloy_provider::{ProviderBuilder, RootProvider, Provider};
/// # async fn test<W: NetworkWallet<Ethereum> + Clone>(url: url::Url, wallet: W) -> Result<(), Box<dyn std::error::Error>> {
/// let provider = ProviderBuilder::default()
///     .with_chain_id(1)
///     .wallet(wallet)
///     .on_http(url);
///
/// provider.send_transaction(TransactionRequest::default()).await;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainIdFiller(Arc<OnceLock<ChainId>>);

impl ChainIdFiller {
    /// Create a new [`ChainIdFiller`] with an optional chain ID.
    ///
    /// If a chain ID is provided, it will be used for filling. If a chain ID
    /// is not provided, the filler will attempt to fetch the chain ID from the
    /// provider the first time a transaction is prepared.
    pub fn new(chain_id: Option<ChainId>) -> Self {
        let lock = OnceLock::new();
        if let Some(chain_id) = chain_id {
            lock.set(chain_id).expect("brand new");
        }
        Self(Arc::new(lock))
    }
}

impl<N: Network> TxFiller<N> for ChainIdFiller {
    type Fillable = ChainId;

    fn status(&self, tx: &N::TransactionRequest) -> FillerControlFlow {
        if tx.chain_id().is_some() {
            FillerControlFlow::Finished
        } else {
            FillerControlFlow::Ready
        }
    }

    fn fill_sync(&self, tx: &mut SendableTx<N>) {
        if let Some(chain_id) = self.0.get() {
            if let Some(builder) = tx.as_mut_builder() {
                if builder.chain_id().is_none() {
                    builder.set_chain_id(*chain_id)
                }
            }
        }
    }

    async fn prepare<P>(
        &self,
        provider: &P,
        _tx: &N::TransactionRequest,
    ) -> TransportResult<Self::Fillable>
    where
        P: crate::Provider<N>,
    {
        match self.0.get().copied() {
            Some(chain_id) => Ok(chain_id),
            None => {
                let chain_id = provider.get_chain_id().await?;
                Ok(*self.0.get_or_init(|| chain_id))
            }
        }
    }

    async fn fill(
        &self,
        _fillable: Self::Fillable,
        mut tx: SendableTx<N>,
    ) -> TransportResult<SendableTx<N>> {
        self.fill_sync(&mut tx);
        Ok(tx)
    }
}
