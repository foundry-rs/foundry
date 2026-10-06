use crate::mining::MinerRequest;
use alloy_primitives::B256;
use alloy_rpc_types_engine::ForkchoiceState;
use eyre::{OptionExt, Result, ensure};
use reth_ethereum::{
    node::api::{
        BuiltPayload, ConsensusEngineHandle, NodePrimitives, PayloadAttributesBuilder, PayloadKind,
        PayloadTypes,
    },
    primitives::SealedHeader,
};
use reth_payload_builder::PayloadBuilderHandle;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;

/// The header type of the blocks a payload type builds.
pub type PayloadHeader<T> =
    <<<T as PayloadTypes>::BuiltPayload as BuiltPayload>::Primitives as NodePrimitives>::BlockHeader;

/// Rewinds the chain to the given header. See [`AnvilMiner::new`].
pub type RewindFn<H> = Box<dyn Fn(&SealedHeader<H>) -> Result<()> + Send + Sync>;

/// Builds blocks on demand through the engine API and keeps track of the chain head.
///
/// Unlike reth's local miner, this miner never finalizes blocks, so a snapshot revert can rewind
/// the head to any earlier block.
pub struct AnvilMiner<T: PayloadTypes> {
    engine: ConsensusEngineHandle<T>,
    payload_builder: PayloadBuilderHandle<T>,
    attributes: Box<dyn PayloadAttributesBuilder<T::PayloadAttributes, PayloadHeader<T>>>,
    map_attributes: Box<dyn Fn(T::PayloadAttributes) -> T::PayloadAttributes + Send + Sync>,
    rewind: RewindFn<PayloadHeader<T>>,
    /// Runs after a payload is built and before the engine validates it.
    before_insert: Box<dyn Fn() -> Result<()> + Send + Sync>,
    last_header: SealedHeader<PayloadHeader<T>>,
    requests: UnboundedReceiver<MinerRequest<PayloadHeader<T>>>,
}

impl<T: PayloadTypes> AnvilMiner<T> {
    /// Creates a miner that extends the chain from `head`.
    ///
    /// `rewind` rewinds the chain state to a header; the miner runs it between blocks so a rewind
    /// never races a block build. `before_insert` runs after a payload is built and before the
    /// engine validates it.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        engine: ConsensusEngineHandle<T>,
        payload_builder: PayloadBuilderHandle<T>,
        attributes: impl PayloadAttributesBuilder<T::PayloadAttributes, PayloadHeader<T>>,
        map_attributes: impl Fn(T::PayloadAttributes) -> T::PayloadAttributes + Send + Sync + 'static,
        rewind: impl Fn(&SealedHeader<PayloadHeader<T>>) -> Result<()> + Send + Sync + 'static,
        before_insert: impl Fn() -> Result<()> + Send + Sync + 'static,
        head: SealedHeader<PayloadHeader<T>>,
        requests: UnboundedReceiver<MinerRequest<PayloadHeader<T>>>,
    ) -> Self {
        Self {
            engine,
            payload_builder,
            attributes: Box::new(attributes),
            map_attributes: Box::new(map_attributes),
            rewind: Box::new(rewind),
            before_insert: Box::new(before_insert),
            last_header: head,
            requests,
        }
    }

    /// Serves requests until the controller drops.
    pub async fn run(mut self) {
        while let Some(request) = self.requests.recv().await {
            match request {
                MinerRequest::Mine(responder) => {
                    let result = self.advance().await;
                    match responder {
                        Some(tx) => {
                            let _ = tx.send(result.map_err(|error| error.to_string()));
                        }
                        None => {
                            if let Err(error) = result {
                                error!(target: "reth_anvil::miner", %error, "failed to mine block");
                            }
                        }
                    }
                }
                MinerRequest::Rewind(header, tx) => {
                    let result = (self.rewind)(&header).map(|()| self.last_header = *header);
                    let _ = tx.send(result.map_err(|error| error.to_string()));
                }
            }
        }
    }

    fn forkchoice_state(&self) -> ForkchoiceState {
        ForkchoiceState {
            head_block_hash: self.last_header.hash(),
            safe_block_hash: B256::ZERO,
            finalized_block_hash: B256::ZERO,
        }
    }

    /// Builds one block on the current head and makes it canonical.
    ///
    /// After a rewind the engine still sees the old head. The forkchoice update then targets a
    /// canonical ancestor with payload attributes, which the engine serves by building on that
    /// ancestor; inserting the built block reorgs the engine onto the rewound chain.
    async fn advance(&mut self) -> Result<SealedHeader<PayloadHeader<T>>> {
        let attributes = (self.map_attributes)(self.attributes.build(&self.last_header));
        let response =
            self.engine.fork_choice_updated(self.forkchoice_state(), Some(attributes)).await?;
        ensure!(response.is_valid(), "forkchoice update rejected: {response:?}");
        let payload_id =
            response.payload_id.ok_or_eyre("forkchoice update returned no payload id")?;

        let Some(Ok(payload)) =
            self.payload_builder.resolve_kind(payload_id, PayloadKind::WaitForPending).await
        else {
            eyre::bail!("payload {payload_id} was not built");
        };
        let header = payload.block().sealed_header().clone();

        (self.before_insert)()?;
        let status = self.engine.new_payload(payload.into()).await?;
        ensure!(status.is_valid(), "payload rejected: {status:?}");
        self.last_header = header.clone();

        let response = self.engine.fork_choice_updated(self.forkchoice_state(), None).await?;
        ensure!(response.is_valid(), "forkchoice update rejected: {response:?}");
        Ok(header)
    }
}
