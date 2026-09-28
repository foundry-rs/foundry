//! Exact Ethereum RPC forks for evm2 execution.

use super::{EthereumEnv, ForkState, fork_db};
use crate::{fork::ResolvedFork, opts::EvmOpts};
use alloy_consensus::BlockHeader;
use alloy_eips::eip7840::BlobParams;
use alloy_network::AnyNetwork;
use alloy_primitives::U256;
use alloy_provider::{Provider, network::BlockResponse};
use evm2::{SpecId, env::BlockEnvExt};
use foundry_config::Config;

/// An exact fork and its accepted state.
#[derive(Clone, Debug)]
pub struct EthereumFork {
    pub env: EthereumEnv,
    pub state: ForkState,
}

impl EthereumFork {
    /// Opens a resolved Ethereum fork.
    pub async fn open(
        config: &Config,
        opts: &EvmOpts,
        resolved: &ResolvedFork,
    ) -> eyre::Result<Self> {
        let provider = opts.provider_for_resolved_fork::<AnyNetwork>(resolved)?;
        opts.ensure_resolved_fork_endpoint(&provider, resolved).await?;
        let block = provider
            .get_block_by_hash(resolved.hash())
            .await?
            .ok_or_else(|| eyre::eyre!("resolved fork block is unavailable"))?;
        let header = block.header();
        eyre::ensure!(
            header.number() == resolved.number() && header.hash == resolved.hash(),
            "resolved fork block changed"
        );
        opts.ensure_resolved_fork_endpoint(&provider, resolved).await?;

        let mut env = EthereumEnv::local_from_config(config, opts)?;
        env.version.chain_id = opts.env.chain_id.unwrap_or(resolved.context().execution_chain_id);
        env.gas_price = U256::from(match opts.env.gas_price {
            Some(gas_price) => gas_price as u128,
            None => provider.get_gas_price().await?,
        });
        let blob_params = match env.spec {
            spec if spec >= SpecId::OSAKA => BlobParams::osaka(),
            spec if spec >= SpecId::PRAGUE => BlobParams::prague(),
            _ => BlobParams::cancun(),
        };
        env.block = BlockEnvExt {
            number: U256::from(header.number()),
            beneficiary: header.beneficiary(),
            timestamp: U256::from(header.timestamp()),
            gas_limit: U256::from(header.gas_limit()),
            basefee: U256::from(header.base_fee_per_gas().unwrap_or_default()),
            difficulty: header.difficulty(),
            prevrandao: U256::from_be_slice(header.mix_hash().unwrap_or_default().as_slice()),
            blob_basefee: U256::from(BlockHeader::blob_fee(header, blob_params).unwrap_or(1)),
            slot_num: U256::from(header.slot_number().unwrap_or_default()),
            ..Default::default()
        };

        let url = opts.fork_url.as_deref().expect("a resolved fork requires a configured URL");
        let meta = fork_db::cache::BlockchainDbMeta::new(serde_json::Value::Null, url.to_owned())
            .with_fork_identity(resolved.hash(), resolved.source_id());
        let db = fork_db::BlockchainDb::new(meta, None);
        let anchor = fork_db::ForkBlock::with_rpc_number(
            header.number(),
            resolved.number(),
            resolved.hash(),
        );
        let (backend, handler) = fork_db::SharedBackend::new_with_anchor(provider, db, anchor)?;
        tokio::spawn(handler);
        Ok(Self { env, state: ForkState::new(backend) })
    }
}
