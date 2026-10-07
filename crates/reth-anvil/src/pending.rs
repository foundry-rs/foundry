//! The pending block environment: the next block as the miner would build it, so calls against
//! the pending block see anvil's next timestamp, coinbase, and prevrandao.

use crate::{block_env::BlockEnvOverrides, evm::AnvilNextBlockEnv, time::TimeManager};
use alloy_network::Ethereum;
use alloy_rpc_types_eth::BlockOverrides;
use reth_ethereum::{
    chainspec::{EthereumHardforks, Hardforks},
    evm::primitives::ConfigureEvm,
    node::{
        api::{FullNodeComponents, HeaderTy, NodeTypes, PrimitivesTy, TxTy},
        builder::rpc::{EthApiBuilder, EthApiCtx},
    },
    primitives::{NodePrimitives, SealedHeader},
    rpc::eth::{EthApiError, EthApiFor, core::EthRpcConverterFor},
};
use reth_rpc_eth_api::{
    FromEvmError, RpcConvert, RpcTypes, SignableTxRequest,
    helpers::pending_block::{BuildPendingEnv, PendingEnvBuilder},
};

/// Builds the environment of the pending block from the time manager and the block environment
/// overrides, as the miner does for the next block.
#[derive(Clone, Debug)]
pub struct AnvilPendingEnv {
    time: TimeManager,
    block_env: BlockEnvOverrides,
}

impl AnvilPendingEnv {
    /// Creates the builder over the node's time manager and block environment overrides.
    pub const fn new(time: TimeManager, block_env: BlockEnvOverrides) -> Self {
        Self { time, block_env }
    }
}

impl<Evm> PendingEnvBuilder<Evm> for AnvilPendingEnv
where
    Evm: ConfigureEvm<
        NextBlockEnvCtx: BuildPendingEnv<<Evm::Primitives as NodePrimitives>::BlockHeader>
                             + AnvilNextBlockEnv,
    >,
{
    fn pending_env_attributes(
        &self,
        parent: &SealedHeader<<Evm::Primitives as NodePrimitives>::BlockHeader>,
        block_overrides: Option<&BlockOverrides>,
    ) -> Result<Evm::NextBlockEnvCtx, EthApiError> {
        let mut attributes = Evm::NextBlockEnvCtx::build_pending_env(parent, block_overrides);
        attributes.set_timestamp(self.time.current_call_timestamp());
        if let Some(coinbase) = self.block_env.coinbase() {
            attributes.set_suggested_fee_recipient(coinbase);
        }
        if let Some(prev_randao) = self.block_env.next_prev_randao() {
            attributes.set_prev_randao(prev_randao);
        }
        Ok(attributes)
    }
}

/// Builds reth's Ethereum `eth` API with [`AnvilPendingEnv`] as the pending block environment.
#[derive(Clone, Debug)]
pub struct AnvilEthApiBuilder {
    pending: AnvilPendingEnv,
}

impl AnvilEthApiBuilder {
    /// Creates the builder over the node's time manager and block environment overrides.
    pub const fn new(time: TimeManager, block_env: BlockEnvOverrides) -> Self {
        Self { pending: AnvilPendingEnv::new(time, block_env) }
    }
}

impl Default for AnvilEthApiBuilder {
    fn default() -> Self {
        Self::new(TimeManager::new(0), BlockEnvOverrides::default())
    }
}

impl<N> EthApiBuilder<N> for AnvilEthApiBuilder
where
    N: FullNodeComponents<
            Types: NodeTypes<ChainSpec: Hardforks + EthereumHardforks>,
            Evm: ConfigureEvm<
                NextBlockEnvCtx: BuildPendingEnv<HeaderTy<N::Types>> + AnvilNextBlockEnv,
            >,
        >,
    Ethereum: RpcTypes<TransactionRequest: SignableTxRequest<TxTy<N::Types>>>,
    EthRpcConverterFor<N, Ethereum>: RpcConvert<
            Primitives = PrimitivesTy<N::Types>,
            Error = EthApiError,
            Network = Ethereum,
            Evm = N::Evm,
        >,
    EthApiError: FromEvmError<N::Evm>,
{
    type EthApi = EthApiFor<N, Ethereum>;

    async fn build_eth_api(self, ctx: EthApiCtx<'_, N>) -> eyre::Result<Self::EthApi> {
        Ok(ctx
            .eth_api_builder()
            .map_converter(|converter| converter.with_network())
            .with_pending_env_builder(self.pending)
            .build())
    }
}
