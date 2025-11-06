use crate::{AsEnvMut, Env, EvmEnv, utils::apply_chain_and_block_specific_env_changes};
use alloy_consensus::BlockHeader;
use alloy_primitives::{Address, U256};
use alloy_provider::{Network, Provider, network::BlockResponse};
use alloy_rpc_types::BlockNumberOrTag;
use foundry_common::NON_ARCHIVE_NODE_WARNING;
use foundry_evm_networks::NetworkConfigs;
use revm::context::{BlockEnv, CfgEnv, TxEnv};

/// Initializes a REVM block environment based on a forked
/// ethereum provider.
#[allow(clippy::too_many_arguments)]
pub async fn environment<N: Network, P: Provider<N>>(
    provider: &P,
    memory_limit: u64,
    override_gas_price: Option<u128>,
    override_chain_id: Option<u64>,
    pin_block: Option<u64>,
    origin: Address,
    disable_block_gas_limit: bool,
    enable_tx_gas_limit: bool,
    configs: NetworkConfigs,
) -> eyre::Result<(Env, N::BlockResponse)> {
    trace!(
        %memory_limit,
        ?override_gas_price,
        ?override_chain_id,
        ?pin_block,
        %origin,
        %disable_block_gas_limit,
        %enable_tx_gas_limit,
        ?configs,
        "creating fork environment"
    );
    let bn = match pin_block {
        Some(bn) => BlockNumberOrTag::Number(bn),
        None => BlockNumberOrTag::Latest,
    };
    let (gas_price, chain_id, block) = tokio::try_join!(
        option_try_or_else(override_gas_price, async || provider.get_gas_price().await),
        option_try_or_else(override_chain_id, async || provider.get_chain_id().await),
        provider.get_block_by_number(bn)
    )?;
    let Some(block) = block else {
        let bn_msg = match bn {
            BlockNumberOrTag::Number(bn) => format!("block number: {bn}"),
            bn => format!("{bn} block"),
        };
        let latest_msg = if let Ok(latest_block) = provider.get_block_number().await {
            // If the `eth_getBlockByNumber` call succeeds, but returns null instead of
            // the block, and the block number is less than equal the latest block, then
            // the user is forking from a non-archive node with an older block number.
            if let Some(block_number) = pin_block
                && block_number <= latest_block
            {
                error!("{NON_ARCHIVE_NODE_WARNING}");
            }
            format!("; latest block number: {latest_block}")
        } else {
            Default::default()
        };
        eyre::bail!("failed to get {bn_msg}{latest_msg}");
    };

    let cfg = configure_env(chain_id, memory_limit, disable_block_gas_limit, enable_tx_gas_limit);

    let mut env = Env {
        evm_env: EvmEnv {
            cfg_env: cfg,
            block_env: BlockEnv {
                number: U256::from(block.header().number()),
                timestamp: U256::from(block.header().timestamp()),
                beneficiary: block.header().beneficiary(),
                difficulty: block.header().difficulty(),
                prevrandao: block.header().mix_hash(),
                basefee: block.header().base_fee_per_gas().unwrap_or_default(),
                gas_limit: block.header().gas_limit(),
                ..Default::default()
            },
        },
        tx: TxEnv {
            caller: origin,
            gas_price,
            chain_id: Some(chain_id),
            gas_limit: block.header().gas_limit(),
            ..Default::default()
        },
    };

    apply_chain_and_block_specific_env_changes::<N>(env.as_env_mut(), &block, configs);

    Ok((env, block))
}

async fn option_try_or_else<T, E>(
    option: Option<T>,
    f: impl AsyncFnOnce() -> Result<T, E>,
) -> Result<T, E> {
    if let Some(value) = option { Ok(value) } else { f().await }
}

/// Configures the environment for the given chain id and memory limit.
pub fn configure_env(
    chain_id: u64,
    memory_limit: u64,
    disable_block_gas_limit: bool,
    enable_tx_gas_limit: bool,
) -> CfgEnv {
    let mut cfg = CfgEnv::default();
    cfg.chain_id = chain_id;
    cfg.memory_limit = memory_limit;
    cfg.limit_contract_code_size = Some(usize::MAX);
    // EIP-3607 rejects transactions from senders with deployed code.
    // If EIP-3607 is enabled it can cause issues during fuzz/invariant tests if the caller
    // is a contract. So we disable the check by default.
    cfg.disable_eip3607 = true;
    cfg.disable_block_gas_limit = disable_block_gas_limit;
    cfg.disable_nonce_check = true;
    // By default do not enforce transaction gas limits imposed by Osaka (EIP-7825).
    // Users can opt-in to enable these limits by setting `enable_tx_gas_limit` to true.
    if !enable_tx_gas_limit {
        cfg.tx_gas_limit_cap = Some(u64::MAX);
    }
    cfg
}
