use ethers::{
    providers::Middleware,
    types::{Address, Block, BlockId, Bytes, TxHash, H256, U256, U64},
};
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    stream::{Fuse, Stream, StreamExt},
    task::{Context, Poll},
    Future,
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

type ProviderRequestFut<Err> = Pin<Box<dyn Future<Output = ProviderResponse<Err>> + Send>>;
type ProviderResult<Ok, Err> =
    Result<OneshotSender<Result<Ok, Err>>, (Err, OneshotSender<Result<Ok, Err>>)>;

/// ProviderHandler internal response type that takes care of the sender until the underlying
/// provider completes the request so that response can be send via the one shot channel.
#[derive(Debug)]
enum ProviderResponse<Err> {
    GetBlockNumber(ProviderResult<U64, Err>),
}

/// The Request type the ProviderHandler listens for
#[derive(Debug)]
enum ProviderRequest<Err> {
    GetBlockNumber(OneshotSender<Result<U64, Err>>),
}

/// Handles an internal provider and listens for commands to delegate to the provider and respond
/// with the provider's response.
#[must_use = "ProviderHandler does nothing unless polled."]
struct ProviderHandler<M: Middleware> {
    provider: M,
    /// Commands that are being processed and awaiting a response from the
    /// provider.
    pending_requests: Vec<ProviderRequestFut<M::Error>>,
    /// Incoming commands
    incoming: Fuse<Receiver<ProviderRequest<M::Error>>>,
}

impl<M: Middleware> ProviderHandler<M> {
    fn new(provider: M, rx: Receiver<ProviderRequest<M::Error>>) -> Self {
        Self { provider, pending_requests: Default::default(), incoming: rx.fuse() }
    }

    fn on_request(&mut self, cmd: ProviderRequest<M::Error>) {
        // TODO execute the correct provider function and store the futre
        dbg!(cmd);
    }
}

impl<M> Future for ProviderHandler<M>
where
    M: Middleware + Unpin,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();

        // receive new commands to delegate to the underlying provider
        while let Poll::Ready(Some(req)) = Pin::new(&mut pin.incoming).poll_next(cx) {
            pin.on_request(req)
        }

        for n in (0..pin.pending_requests.len()).rev() {
            let request = pin.pending_requests.swap_remove(n);
            // TODO poll the in progress requests
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
#[derive(Debug, Clone)]
pub struct SyncProvider<M: Middleware> {
    provider: Sender<ProviderRequest<M::Error>>,
}

impl<M> SyncProvider<M>
where
    M: Middleware + Unpin + 'static,
{
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

    // TODO port all essentiall `Middleware` functions, but sync
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sync_provider_test_poc() {
        use std::sync::mpsc::Sender as SyncSender;
        let (mut tx, mut rx) = channel::<SyncSender<u64>>(1);

        std::thread::spawn(|| {
            let rt = Runtime::new().unwrap();

            rt.block_on(async move {
                let x = rx.next().await.unwrap();
                x.send(69).unwrap();
            });
        });

        let (tx2, rx2) = std::sync::mpsc::channel();
        tx.try_send(tx2).unwrap();
        let received = rx2.recv().unwrap();
        assert_eq!(received, 69)
    }
}
