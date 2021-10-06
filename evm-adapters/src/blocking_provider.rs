use ethers::{
    providers::Middleware,
    types::{
        transaction::eip2718::TypedTransaction, Address, Block, BlockId, BlockNumber, Bytes,
        EIP1186ProofResponse, NameOrAddress, Transaction, TransactionReceipt, TxHash, H256, U256,
        U64,
    },
};
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    stream::{Fuse, Stream, StreamExt},
    task::{Context, Poll},
    Future, FutureExt,
};
use std::{
    pin::Pin,
    sync::mpsc::{channel as oneshot_channel, Sender as OneshotSender},
};
use tokio::runtime::Runtime;

#[derive(Debug)]
/// Blocking wrapper around an Ethers middleware, for use in synchronous contexts
/// (powered by a tokio runtime)
pub struct BlockingProvider<M> {
    provider: M,
    runtime: Runtime,
}

impl<M: Clone> Clone for BlockingProvider<M> {
    fn clone(&self) -> Self {
        Self { provider: self.provider.clone(), runtime: Runtime::new().unwrap() }
    }
}

impl<M: Middleware> BlockingProvider<M>
where
    M::Error: 'static,
{
    pub fn new(provider: M) -> Self {
        Self { provider, runtime: Runtime::new().unwrap() }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.runtime.block_on(f)
    }

    pub fn block_and_chainid(&self, block_id: BlockId) -> eyre::Result<(Block<TxHash>, U256)> {
        let f = async {
            let block = self.provider.get_block(block_id);
            let chain_id = self.provider.get_chainid();
            tokio::try_join!(block, chain_id)
        };
        let (block, chain_id) = self.block_on(f)?;
        Ok((block.ok_or_else(|| eyre::eyre!("block {:?} not found", block_id))?, chain_id))
    }

    pub fn get_account(
        &self,
        address: Address,
        block_id: Option<BlockId>,
    ) -> eyre::Result<(U256, U256, Bytes)> {
        let f = async {
            let balance = self.provider.get_balance(address, block_id);
            let nonce = self.provider.get_transaction_count(address, block_id);
            let code = self.provider.get_code(address, block_id);
            tokio::try_join!(balance, nonce, code)
        };
        let (balance, nonce, code) = self.block_on(f)?;

        Ok((nonce, balance, code))
    }

    pub fn get_block_number(&self) -> Result<U64, M::Error> {
        self.block_on(self.provider.get_block_number())
    }

    pub fn get_balance(&self, address: Address, block: Option<BlockId>) -> Result<U256, M::Error> {
        self.block_on(self.provider.get_balance(address, block))
    }

    pub fn get_accounts(&self) -> Result<Vec<Address>, M::Error> {
        self.block_on(self.provider.get_accounts())
    }

    pub fn get_transaction_count(
        &self,
        address: Address,
        block: Option<BlockId>,
    ) -> Result<U256, M::Error> {
        self.block_on(self.provider.get_transaction_count(address, block))
    }

    pub fn get_code(&self, address: Address, block: Option<BlockId>) -> Result<Bytes, M::Error> {
        self.block_on(self.provider.get_code(address, block))
    }

    pub fn get_storage_at(
        &self,
        address: Address,
        slot: H256,
        block: Option<BlockId>,
    ) -> Result<H256, M::Error> {
        self.block_on(self.provider.get_storage_at(address, slot, block))
    }
}

/// The Request type the ProviderHandler listens for
#[derive(Debug)]
enum ProviderRequest<Err> {
    GetBlockNumber(OneshotSender<Result<U64, Err>>),
    ResolveName {
        ens_name: String,
        sender: OneshotSender<Result<Address, Err>>,
    },
    LookupAddress {
        address: Address,
        sender: OneshotSender<Result<String, Err>>,
    },
    GetBlock {
        block: BlockId,
        sender: OneshotSender<Result<Option<Block<TxHash>>, Err>>,
    },
    GetBlockWithTxs {
        block: BlockId,
        sender: OneshotSender<Result<Option<Block<Transaction>>, Err>>,
    },
    GetUncleCount {
        block_hash_or_number: BlockId,
        sender: OneshotSender<Result<U256, Err>>,
    },
    GetUncle {
        block: BlockId,
        idx: U64,
        sender: OneshotSender<Result<Option<Block<H256>>, Err>>,
    },
    GetTransactionCount {
        from: NameOrAddress,
        block: Option<BlockId>,
        sender: OneshotSender<Result<U256, Err>>,
    },
    EstimateGas {
        tx: TypedTransaction,
        sender: OneshotSender<Result<U256, Err>>,
    },
    Call {
        tx: TypedTransaction,
        block: Option<BlockId>,
        sender: OneshotSender<Result<Bytes, Err>>,
    },
    GetChainId(OneshotSender<Result<U256, Err>>),
    GetBalance {
        from: NameOrAddress,
        block: Option<BlockId>,
        sender: OneshotSender<Result<U256, Err>>,
    },
    GetTransaction {
        transaction_hash: TxHash,
        sender: OneshotSender<Result<Option<Transaction>, Err>>,
    },
    GetTransactionReceipt {
        transaction_hash: TxHash,
        sender: OneshotSender<Result<Option<TransactionReceipt>, Err>>,
    },
    GetBlockReceipts {
        block: BlockNumber,
        sender: OneshotSender<Result<Vec<TransactionReceipt>, Err>>,
    },
    GetGasPrice(OneshotSender<Result<U256, Err>>),
    GetAccounts(OneshotSender<Result<Vec<Address>, Err>>),
    GetStorageAt {
        from: NameOrAddress,
        location: H256,
        block: Option<BlockId>,
        sender: OneshotSender<Result<H256, Err>>,
    },
    GetProof {
        from: NameOrAddress,
        locations: Vec<H256>,
        block: Option<BlockId>,
        sender: OneshotSender<Result<EIP1186ProofResponse, Err>>,
    },
}

type ProviderRequestFut = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Handles an internal provider and listens for commands to delegate to the provider and respond
/// with the provider's response.
#[must_use = "ProviderHandler does nothing unless polled."]
struct ProviderHandler<M: Middleware> {
    provider: M,
    /// Commands that are being processed and awaiting a response from the
    /// provider.
    pending_requests: Vec<ProviderRequestFut>,
    /// Incoming commands
    incoming: Fuse<Receiver<ProviderRequest<M::Error>>>,
}

impl<M> ProviderHandler<M>
where
    M: Middleware + Clone + 'static,
{
    fn new(provider: M, rx: Receiver<ProviderRequest<M::Error>>) -> Self {
        Self { provider, pending_requests: Default::default(), incoming: rx.fuse() }
    }

    /// handle the request in queue in the future
    fn on_request(&mut self, cmd: ProviderRequest<M::Error>) {
        let provider = self.provider.clone();
        let fut = Box::pin(async move {
            match cmd {
                ProviderRequest::GetBlockNumber(sender) => {
                    let resp = provider.get_block_number().await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::ResolveName { ens_name, sender } => {
                    let resp = provider.resolve_name(&ens_name).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::LookupAddress { address, sender } => {
                    let resp = provider.lookup_address(address).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetBlock { block, sender } => {
                    let resp = provider.get_block(block).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetBlockWithTxs { block, sender } => {
                    let resp = provider.get_block_with_txs(block).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetUncleCount { block_hash_or_number, sender } => {
                    let resp = provider.get_uncle_count(block_hash_or_number).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetUncle { block, idx, sender } => {
                    let resp = provider.get_uncle(block, idx).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetTransactionCount { from, block, sender } => {
                    let resp = provider.get_transaction_count(from, block).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::EstimateGas { tx, sender } => {
                    let resp = provider.estimate_gas(&tx).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::Call { tx, block, sender } => {
                    let resp = provider.call(&tx, block).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetChainId(sender) => {
                    let resp = provider.get_chainid().await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetBalance { from, block, sender } => {
                    let resp = provider.get_balance(from, block).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetTransaction { transaction_hash, sender } => {
                    let resp = provider.get_transaction(transaction_hash).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetTransactionReceipt { transaction_hash, sender } => {
                    let resp = provider.get_transaction_receipt(transaction_hash).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetBlockReceipts { block, sender } => {
                    let resp = provider.get_block_receipts(block).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetGasPrice(sender) => {
                    let resp = provider.get_gas_price().await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetAccounts(sender) => {
                    let resp = provider.get_accounts().await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetStorageAt { from, location, block, sender } => {
                    let resp = provider.get_storage_at(from, location, block).await;
                    let _ = sender.send(resp);
                }
                ProviderRequest::GetProof { from, locations, block, sender } => {
                    let resp = provider.get_proof(from, locations, block).await;
                    let _ = sender.send(resp);
                }
            }
        });
        self.pending_requests.push(fut);
    }
}

impl<M> Future for ProviderHandler<M>
where
    M: Middleware + Clone + Unpin + 'static,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        // receive new commands to delegate to the underlying provider
        while let Poll::Ready(Some(req)) = Pin::new(&mut pin.incoming).poll_next(cx) {
            pin.on_request(req)
        }

        // poll all futures
        for n in (0..pin.pending_requests.len()).rev() {
            let mut request = pin.pending_requests.swap_remove(n);
            if request.poll_unpin(cx).is_pending() {
                pin.pending_requests.push(request);
            }
        }

        // the handler is finished if the command channel was closed and all commands are processed
        if pin.incoming.is_done() && pin.pending_requests.is_empty() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// A blocking alternative to the async `Middleware`.
#[derive(Debug)]
pub struct SyncProvider<M: Middleware> {
    provider: Sender<ProviderRequest<M::Error>>,
}

impl<M: Middleware> Clone for SyncProvider<M> {
    fn clone(&self) -> Self {
        Self { provider: self.provider.clone() }
    }
}

impl<M> SyncProvider<M>
where
    M: Middleware + Unpin + 'static + Clone,
{
    /// NOTE: this should be called with `Arc<Provider>`
    pub fn new(provider: M) -> eyre::Result<Self> {
        let (tx, rx) = channel(1);
        let handler = ProviderHandler::new(provider, rx);
        // spawn the provider handler to background for which we need a new Runtime
        let rt = Runtime::new()?;
        std::thread::spawn(move || rt.block_on(handler));

        Ok(Self { provider: tx })
    }

    pub fn get_block_number(&self) -> eyre::Result<U64> {
        let (tx, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetBlockNumber(tx);
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn resolve_name(&self, ens_name: &str) -> eyre::Result<Address> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::ResolveName { ens_name: ens_name.to_string(), sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn lookup_address(&self, address: Address) -> eyre::Result<String> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::LookupAddress { address, sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_block<T: Into<BlockId>>(&self, block: T) -> eyre::Result<Option<Block<TxHash>>> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetBlock { block: block.into(), sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_block_with_txs<T: Into<BlockId>>(
        &self,
        block: T,
    ) -> eyre::Result<Option<Block<Transaction>>> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetBlockWithTxs { block: block.into(), sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_uncle_count<T: Into<BlockId>>(&self, block: T) -> eyre::Result<U256> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetUncleCount { block_hash_or_number: block.into(), sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_uncle<T: Into<BlockId>>(
        &self,
        block: T,
        idx: U64,
    ) -> eyre::Result<Option<Block<H256>>> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetUncle { block: block.into(), idx, sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_transaction_count<T: Into<NameOrAddress>>(
        &self,
        from: T,
        block: Option<BlockId>,
    ) -> eyre::Result<U256> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetTransactionCount { from: from.into(), block, sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn estimate_gas(&self, tx: TypedTransaction) -> eyre::Result<U256> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::EstimateGas { tx, sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn call(&self, tx: TypedTransaction, block: Option<BlockId>) -> eyre::Result<Bytes> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::Call { tx, block, sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_chainid(&self) -> eyre::Result<U256> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetChainId(sender);
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_balance<T: Into<NameOrAddress>>(
        &self,
        from: T,
        block: Option<BlockId>,
    ) -> eyre::Result<U256> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetBalance { from: from.into(), block, sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_transaction<T: Into<TxHash>>(
        &self,
        transaction_hash: T,
    ) -> eyre::Result<Option<Transaction>> {
        let (sender, rx) = oneshot_channel();
        let cmd =
            ProviderRequest::GetTransaction { transaction_hash: transaction_hash.into(), sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_transaction_receipt<T: Into<TxHash>>(
        &self,
        transaction_hash: T,
    ) -> eyre::Result<Option<TransactionReceipt>> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetTransactionReceipt {
            transaction_hash: transaction_hash.into(),
            sender,
        };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_block_receipts<T: Into<BlockNumber>>(
        &self,
        block: T,
    ) -> eyre::Result<Vec<TransactionReceipt>> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetBlockReceipts { block: block.into(), sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_gas_price(&self) -> eyre::Result<U256> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetGasPrice(sender);
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_accounts(&self) -> eyre::Result<Vec<Address>> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetAccounts(sender);
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_storage_at<T: Into<NameOrAddress>>(
        &self,
        from: T,
        location: H256,
        block: Option<BlockId>,
    ) -> eyre::Result<H256> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetStorageAt { from: from.into(), location, block, sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }

    pub fn get_proof<T: Into<NameOrAddress>>(
        &self,
        from: T,
        locations: Vec<H256>,
        block: Option<BlockId>,
    ) -> eyre::Result<EIP1186ProofResponse> {
        let (sender, rx) = oneshot_channel();
        let cmd = ProviderRequest::GetProof { from: from.into(), locations, block, sender };
        self.provider.clone().try_send(cmd).map_err(|e| eyre::eyre!("{:?}", e))?;
        Ok(rx.recv()??)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethers::{
        providers::{Http, Provider},
        utils::Ganache,
    };
    use std::{convert::TryFrom, sync::Arc};

    #[test]
    fn sync_provider_test_poc() {
        let ganache = Ganache::new().spawn();

        // connect to the network
        let provider = Provider::<Http>::try_from(ganache.endpoint()).unwrap();

        let provider = SyncProvider::new(Arc::new(provider)).unwrap();

        let _ = provider.get_accounts().unwrap();
    }
}
