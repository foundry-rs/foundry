use std::future::IntoFuture;

use crate::{
    fillers::{FillerControlFlow, TxFiller},
    provider::SendableTx,
    utils::Eip1559Estimation,
    Provider,
};
use alloy_eips::eip4844::BLOB_TX_MIN_BLOB_GASPRICE;
use alloy_json_rpc::RpcError;
use alloy_network::{Network, TransactionBuilder, TransactionBuilder4844};
use alloy_rpc_types_eth::BlockNumberOrTag;
use alloy_transport::TransportResult;
use futures::FutureExt;

/// An enum over the different types of gas fillable.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GasFillable {
    Legacy { gas_limit: u64, gas_price: u128 },
    Eip1559 { gas_limit: u64, estimate: Eip1559Estimation },
}

/// A [`TxFiller`] that populates gas related fields in transaction requests if
/// unset.
///
/// Gas related fields are gas_price, gas_limit, max_fee_per_gas
/// max_priority_fee_per_gas and max_fee_per_blob_gas.
///
/// The layer fetches the estimations for these via the
/// [`Provider::get_gas_price`], [`Provider::estimate_gas`] and
/// [`Provider::estimate_eip1559_fees`] methods.
///
/// ## Note:
///
/// The layer will populate gas fields based on the following logic:
/// - if `gas_price` is set, it will process as a legacy tx and populate the `gas_limit` field if
///   unset.
/// - if `access_list` is set, it will process as a 2930 tx and populate the `gas_limit` and
///   `gas_price` field if unset.
/// - if `blob_sidecar` is set, it will process as a 4844 tx and populate the `gas_limit`,
///   `max_fee_per_gas`, `max_priority_fee_per_gas` and `max_fee_per_blob_gas` fields if unset.
/// - Otherwise, it will process as a EIP-1559 tx and populate the `gas_limit`, `max_fee_per_gas`
///   and `max_priority_fee_per_gas` fields if unset.
/// - If the network does not support EIP-1559, it will fallback to the legacy tx and populate the
///   `gas_limit` and `gas_price` fields if unset.
///
/// # Example
///
/// ```
/// # use alloy_network::{NetworkWallet, EthereumWallet, Ethereum};
/// # use alloy_rpc_types_eth::TransactionRequest;
/// # use alloy_provider::{ProviderBuilder, RootProvider, Provider};
/// # async fn test<W: NetworkWallet<Ethereum> + Clone>(url: url::Url, wallet: W) -> Result<(), Box<dyn std::error::Error>> {
/// let provider = ProviderBuilder::default()
///     .with_gas_estimation()
///     .wallet(wallet)
///     .on_http(url);
///
/// provider.send_transaction(TransactionRequest::default()).await;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct GasFiller;

impl GasFiller {
    async fn prepare_legacy<P, N>(
        &self,
        provider: &P,
        tx: &N::TransactionRequest,
    ) -> TransportResult<GasFillable>
    where
        P: Provider<N>,
        N: Network,
    {
        let gas_price_fut = tx.gas_price().map_or_else(
            || provider.get_gas_price().right_future(),
            |gas_price| async move { Ok(gas_price) }.left_future(),
        );

        let gas_limit_fut = tx.gas_limit().map_or_else(
            || provider.estimate_gas(tx).into_future().right_future(),
            |gas_limit| async move { Ok(gas_limit) }.left_future(),
        );

        let (gas_price, gas_limit) = futures::try_join!(gas_price_fut, gas_limit_fut)?;

        Ok(GasFillable::Legacy { gas_limit, gas_price })
    }

    async fn prepare_1559<P, N>(
        &self,
        provider: &P,
        tx: &N::TransactionRequest,
    ) -> TransportResult<GasFillable>
    where
        P: Provider<N>,
        N: Network,
    {
        let gas_limit_fut = tx.gas_limit().map_or_else(
            || provider.estimate_gas(tx).into_future().right_future(),
            |gas_limit| async move { Ok(gas_limit) }.left_future(),
        );

        let eip1559_fees_fut = if let (Some(max_fee_per_gas), Some(max_priority_fee_per_gas)) =
            (tx.max_fee_per_gas(), tx.max_priority_fee_per_gas())
        {
            async move { Ok(Eip1559Estimation { max_fee_per_gas, max_priority_fee_per_gas }) }
                .left_future()
        } else {
            provider.estimate_eip1559_fees(None).right_future()
        };

        let (gas_limit, estimate) = futures::try_join!(gas_limit_fut, eip1559_fees_fut)?;

        Ok(GasFillable::Eip1559 { gas_limit, estimate })
    }
}

impl<N: Network> TxFiller<N> for GasFiller {
    type Fillable = GasFillable;

    fn status(&self, tx: &<N as Network>::TransactionRequest) -> FillerControlFlow {
        // legacy and eip2930 tx
        if tx.gas_price().is_some() && tx.gas_limit().is_some() {
            return FillerControlFlow::Finished;
        }

        // eip1559
        if tx.max_fee_per_gas().is_some()
            && tx.max_priority_fee_per_gas().is_some()
            && tx.gas_limit().is_some()
        {
            return FillerControlFlow::Finished;
        }

        FillerControlFlow::Ready
    }

    fn fill_sync(&self, _tx: &mut SendableTx<N>) {}

    async fn prepare<P>(
        &self,
        provider: &P,
        tx: &<N as Network>::TransactionRequest,
    ) -> TransportResult<Self::Fillable>
    where
        P: Provider<N>,
    {
        if tx.gas_price().is_some() {
            self.prepare_legacy(provider, tx).await
        } else {
            match self.prepare_1559(provider, tx).await {
                // fallback to legacy
                Ok(estimate) => Ok(estimate),
                Err(RpcError::UnsupportedFeature(_)) => self.prepare_legacy(provider, tx).await,
                Err(e) => Err(e),
            }
        }
    }

    async fn fill(
        &self,
        fillable: Self::Fillable,
        mut tx: SendableTx<N>,
    ) -> TransportResult<SendableTx<N>> {
        if let Some(builder) = tx.as_mut_builder() {
            match fillable {
                GasFillable::Legacy { gas_limit, gas_price } => {
                    builder.set_gas_limit(gas_limit);
                    builder.set_gas_price(gas_price);
                }
                GasFillable::Eip1559 { gas_limit, estimate } => {
                    builder.set_gas_limit(gas_limit);
                    builder.set_max_fee_per_gas(estimate.max_fee_per_gas);
                    builder.set_max_priority_fee_per_gas(estimate.max_priority_fee_per_gas);
                }
            }
        };
        Ok(tx)
    }
}

/// Filler for the `max_fee_per_blob_gas` field in EIP-4844 transactions.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlobGasFiller;

impl<N: Network> TxFiller<N> for BlobGasFiller
where
    N::TransactionRequest: TransactionBuilder4844,
{
    type Fillable = u128;

    fn status(&self, tx: &<N as Network>::TransactionRequest) -> FillerControlFlow {
        // Nothing to fill if non-eip4844 tx or `max_fee_per_blob_gas` is already set to a valid
        // value.
        if tx.blob_sidecar().is_none()
            || tx.max_fee_per_blob_gas().is_some_and(|gas| gas >= BLOB_TX_MIN_BLOB_GASPRICE)
        {
            return FillerControlFlow::Finished;
        }

        FillerControlFlow::Ready
    }

    fn fill_sync(&self, _tx: &mut SendableTx<N>) {}

    async fn prepare<P>(
        &self,
        provider: &P,
        tx: &<N as Network>::TransactionRequest,
    ) -> TransportResult<Self::Fillable>
    where
        P: Provider<N>,
    {
        if let Some(max_fee_per_blob_gas) = tx.max_fee_per_blob_gas() {
            if max_fee_per_blob_gas >= BLOB_TX_MIN_BLOB_GASPRICE {
                return Ok(max_fee_per_blob_gas);
            }
        }

        provider
            .get_fee_history(2, BlockNumberOrTag::Latest, &[])
            .await?
            .base_fee_per_blob_gas
            .last()
            .ok_or(RpcError::NullResp)
            .copied()
    }

    async fn fill(
        &self,
        fillable: Self::Fillable,
        mut tx: SendableTx<N>,
    ) -> TransportResult<SendableTx<N>> {
        if let Some(builder) = tx.as_mut_builder() {
            builder.set_max_fee_per_blob_gas(fillable);
        }
        Ok(tx)
    }
}

#[cfg(feature = "reqwest")]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderBuilder;
    use alloy_consensus::{SidecarBuilder, SimpleCoder, Transaction};
    use alloy_eips::eip4844::DATA_GAS_PER_BLOB;
    use alloy_primitives::{address, U256};
    use alloy_rpc_types_eth::TransactionRequest;

    #[tokio::test]
    async fn no_gas_price_or_limit() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        // GasEstimationLayer requires chain_id to be set to handle EIP-1559 tx
        let tx = TransactionRequest {
            value: Some(U256::from(100)),
            to: Some(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into()),
            chain_id: Some(31337),
            ..Default::default()
        };

        let tx = provider.send_transaction(tx).await.unwrap();

        let receipt = tx.get_receipt().await.unwrap();

        assert_eq!(receipt.effective_gas_price, 1_000_000_001);
        assert_eq!(receipt.gas_used, 21000);
    }

    #[tokio::test]
    async fn no_gas_limit() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        let gas_price = provider.get_gas_price().await.unwrap();
        let tx = TransactionRequest {
            value: Some(U256::from(100)),
            to: Some(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into()),
            gas_price: Some(gas_price),
            ..Default::default()
        };

        let tx = provider.send_transaction(tx).await.unwrap();

        let receipt = tx.get_receipt().await.unwrap();

        assert_eq!(receipt.gas_used, 21000);
    }

    #[tokio::test]
    async fn no_max_fee_per_blob_gas() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Hello World");
        let sidecar = sidecar.build().unwrap();

        let tx = TransactionRequest {
            to: Some(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into()),
            sidecar: Some(sidecar),
            ..Default::default()
        };

        let tx = provider.send_transaction(tx).await.unwrap();

        let receipt = tx.get_receipt().await.unwrap();

        let tx = provider.get_transaction_by_hash(receipt.transaction_hash).await.unwrap().unwrap();

        assert!(tx.max_fee_per_blob_gas().unwrap() >= BLOB_TX_MIN_BLOB_GASPRICE);
        assert_eq!(receipt.gas_used, 21000);
        assert_eq!(
            receipt.blob_gas_used.expect("Expected to be EIP-4844 transaction"),
            DATA_GAS_PER_BLOB
        );
    }

    #[tokio::test]
    async fn zero_max_fee_per_blob_gas() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        let sidecar: SidecarBuilder<SimpleCoder> = SidecarBuilder::from_slice(b"Hello World");
        let sidecar = sidecar.build().unwrap();

        let tx = TransactionRequest {
            to: Some(address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").into()),
            max_fee_per_blob_gas: Some(0),
            sidecar: Some(sidecar),
            ..Default::default()
        };

        let tx = provider.send_transaction(tx).await.unwrap();

        let receipt = tx.get_receipt().await.unwrap();

        let tx = provider.get_transaction_by_hash(receipt.transaction_hash).await.unwrap().unwrap();

        assert!(tx.max_fee_per_blob_gas().unwrap() >= BLOB_TX_MIN_BLOB_GASPRICE);
        assert_eq!(receipt.gas_used, 21000);
        assert_eq!(
            receipt.blob_gas_used.expect("Expected to be EIP-4844 transaction"),
            DATA_GAS_PER_BLOB
        );
    }
}
