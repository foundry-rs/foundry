use crate::{evm::EvmEnv, hardfork::FoundryHardfork};
use alloy_chains::Chain;
use alloy_consensus::{BlockHeader, private::alloy_eips::eip7840::BlobParams};
use alloy_hardforks::EthereumHardfork;
use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{ChainId, Selector, U256};
use alloy_provider::{Network, network::BlockResponse};
use evm2::{EvmFeatures, EvmTypesHost, SpecId, env::BlockEnv};
use foundry_config::NamedChain::{
    self, Avalanche, AvalancheFuji, BinanceSmartChain, BinanceSmartChainTestnet, Polygon,
    PolygonAmoy,
};
use foundry_evm_networks::{NetworkConfigs, arbitrum};

/// Hints to the compiler that this is a cold path, i.e. unlikely to be taken.
#[cold]
#[inline(always)]
pub const fn cold_path() {
    // TODO: remove `#[cold]` and call `std::hint::cold_path` once stable.
}

/// Constructs a block environment from a block header.
///
/// A header without `mixHash` leaves `prevrandao` at zero.
pub fn block_env_from_header<T: EvmTypesHost>(header: &impl BlockHeader) -> BlockEnv<T> {
    BlockEnv::<T> {
        number: U256::from(header.number()),
        beneficiary: header.beneficiary(),
        timestamp: U256::from(header.timestamp()),
        gas_limit: U256::from(header.gas_limit()),
        basefee: U256::from(header.base_fee_per_gas().unwrap_or_default()),
        difficulty: header.difficulty(),
        prevrandao: header.mix_hash().unwrap_or_default().into(),
        slot_num: U256::from(header.slot_number().unwrap_or_default()),
        ..Default::default()
    }
}

/// Applies replay normalization for the provided source chain.
///
/// This keeps fork-specific transaction validation independent from an execution `CHAINID`
/// override.
pub fn apply_chain_specific_tx_replay_env_changes_for_chain<T: EvmTypesHost>(
    evm_env: &mut EvmEnv<T>,
    source_chain_id: ChainId,
) {
    let chain = NamedChain::try_from(source_chain_id);
    if chain.is_ok_and(|chain| chain.is_arbitrum()) {
        // Arbitrum does not enforce the EIP-1559 priority fee ordering constraint.
        evm_env.set_feature(EvmFeatures::PRIORITY_FEE_CHECK, false);
    }
    if matches!(chain, Ok(NamedChain::Celo | NamedChain::CeloSepolia)) {
        // CIP-64 fee caps are denominated in the fee currency, not in the native base fee.
        evm_env.set_feature(EvmFeatures::BASE_FEE_CHECK, false);
    }
}

/// Applies block normalization for the provided source chain.
///
/// This keeps fork-specific header handling independent from an execution `CHAINID` override.
pub fn apply_chain_and_block_specific_env_changes_for_chain<N: Network, T: EvmTypesHost>(
    evm_env: &mut EvmEnv<T>,
    block: &N::BlockResponse,
    source_chain_id: ChainId,
    configs: NetworkConfigs,
) {
    let header = block.header();
    // The blob fee market is priced from the header's excess blob gas and the source chain's
    // blob schedule at the block timestamp. Headers without the field (pre-Cancun blocks and
    // chains without EIP-4844) keep the default blob environment.
    if let Some(excess_blob_gas) = header.excess_blob_gas() {
        evm_env.set_blob_excess_gas(
            excess_blob_gas,
            get_blob_base_fee_update_fraction(source_chain_id, header.timestamp()),
        );
    }

    let block_env = &mut evm_env.block;
    if let Ok(chain) = NamedChain::try_from(source_chain_id) {
        match chain {
            BinanceSmartChain
            | BinanceSmartChainTestnet
            | Polygon
            | PolygonAmoy
            | Avalanche
            | AvalancheFuji => {
                // https://github.com/foundry-rs/foundry/issues/9942
                // As far as observed from the source code of bnb-chain/bsc, the `difficulty` field
                // is still in use and returned by the corresponding opcode but `prevrandao`
                // (`mixHash`) is always zero, even though bsc adopts the newer EVM
                // specification. Polygon and Avalanche behave the same way.
                block_env.prevrandao = block_env.difficulty;
                return;
            }
            c if c.is_arbitrum() => {
                // on arbitrum `block.number` is the L1 block which is included in the
                // `l1BlockNumber` field
                if let Some(l1_block_number) = block
                    .other_fields()
                    .and_then(|other| other.get("l1BlockNumber").cloned())
                    .and_then(|l1_block_number| {
                        serde_json::from_value::<U256>(l1_block_number).ok()
                    })
                {
                    block_env.number = l1_block_number;
                }

                // `mixHash` carries L1 metadata rather than randomness here, while the
                // `PREVRANDAO` opcode returns `difficulty` like it does on the chains above.
                block_env.prevrandao = block_env.difficulty;
            }
            _ => {}
        }
    }

    if configs.bypass_prevrandao(source_chain_id) && header.mix_hash().is_none() {
        // Chains without randomness keep the historical DIFFICULTY opcode semantics.
        // <https://github.com/foundry-rs/foundry/issues/4232>
        block_env.prevrandao = block_env.difficulty;
    }

    // if difficulty is `0` we assume it's past merge
    if header.difficulty().is_zero() {
        block_env.difficulty = block_env.prevrandao;
    }
}

/// Derives the active [`BlobParams`] based on the given timestamp.
///
/// This falls back to regular ethereum blob params if no hardforks for the given chain id are
/// detected.
pub fn get_blob_params(chain_id: ChainId, timestamp: u64) -> BlobParams {
    let hardfork = EthereumHardfork::from_chain_and_timestamp(Chain::from_id(chain_id), timestamp)
        .unwrap_or_default();

    match hardfork {
        EthereumHardfork::Prague => BlobParams::prague(),
        EthereumHardfork::Osaka => BlobParams::osaka(),
        EthereumHardfork::Bpo1 => BlobParams::bpo1(),
        EthereumHardfork::Bpo2 => BlobParams::bpo2(),

        // future hardforks/unknown settings: update once decided
        EthereumHardfork::Bpo3 => BlobParams::bpo2(),
        EthereumHardfork::Bpo4 => BlobParams::bpo2(),
        EthereumHardfork::Bpo5 => BlobParams::bpo2(),
        EthereumHardfork::Amsterdam => BlobParams::bpo2(),

        // fallback
        _ => BlobParams::cancun(),
    }
}

/// Derive the blob base fee update fraction based on the chain and timestamp by checking the
/// hardfork.
pub fn get_blob_base_fee_update_fraction(chain_id: ChainId, timestamp: u64) -> u64 {
    get_blob_params(chain_id, timestamp).update_fraction as u64
}

/// Returns the blob params based on the spec id.
pub fn get_blob_params_by_spec_id(spec: SpecId) -> BlobParams {
    if spec >= SpecId::AMSTERDAM {
        BlobParams::bpo2()
    } else if spec >= SpecId::OSAKA {
        BlobParams::osaka()
    } else if spec >= SpecId::PRAGUE {
        BlobParams::prague()
    } else {
        BlobParams::cancun()
    }
}

/// Returns the blob parameters selected by an explicit Foundry hardfork.
pub fn get_blob_params_by_hardfork(hardfork: FoundryHardfork) -> BlobParams {
    match hardfork {
        FoundryHardfork::Ethereum(EthereumHardfork::Prague) => BlobParams::prague(),
        FoundryHardfork::Ethereum(EthereumHardfork::Osaka) => BlobParams::osaka(),
        FoundryHardfork::Ethereum(EthereumHardfork::Bpo1) => BlobParams::bpo1(),
        FoundryHardfork::Ethereum(EthereumHardfork::Bpo2) => BlobParams::bpo2(),
        FoundryHardfork::Ethereum(
            EthereumHardfork::Bpo3
            | EthereumHardfork::Bpo4
            | EthereumHardfork::Bpo5
            | EthereumHardfork::Amsterdam,
        ) => BlobParams::bpo2(),
        _ => get_blob_params_by_spec_id(hardfork.into()),
    }
}

/// Returns the blob base fee update fraction based on the spec id.
pub fn get_blob_base_fee_update_fraction_by_spec_id(spec: SpecId) -> u64 {
    get_blob_params_by_spec_id(spec).update_fraction as u64
}

/// Given an ABI and selector, it tries to find the respective function.
pub fn get_function<'a>(
    contract_name: &str,
    selector: Selector,
    abi: &'a JsonAbi,
) -> eyre::Result<&'a Function> {
    abi.functions()
        .find(|func| func.selector() == selector)
        .ok_or_else(|| eyre::eyre!("{contract_name} does not have the selector {selector}"))
}

/// Returns the addresses a transaction in `env` with `access_list` warms when it starts and that
/// `vm.cool` keeps warm: the EIP-3651 coinbase, Arbitrum's ArbSys on Arbitrum chains, and the
/// access list.
pub fn tx_warm_addresses<T: EvmTypesHost>(
    env: &EvmEnv<T>,
    access_list: Option<&alloy_eips::eip2930::AccessList>,
) -> Vec<alloy_primitives::Address> {
    let coinbase = env.version.feature(EvmFeatures::EIP3651).then_some(env.block.beneficiary);
    let arb_sys =
        arbitrum::is_arbitrum_chain(env.version.chain_id).then_some(arbitrum::ARB_SYS_ADDRESS);
    let access_list = access_list.into_iter().flat_map(|list| list.iter().map(|item| item.address));
    coinbase.into_iter().chain(arb_sys).chain(access_list).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::{AnyHeader, AnyNetwork, AnyRpcBlock, AnyRpcHeader};
    use alloy_primitives::B256;
    use alloy_rpc_types::{Block, BlockTransactions};
    use evm2::{BaseEvmTypes, Version};

    fn env(block: BlockEnv<BaseEvmTypes>) -> EvmEnv<BaseEvmTypes> {
        EvmEnv::new(SpecId::PRAGUE, Version::new(SpecId::PRAGUE), block)
    }

    #[test]
    fn block_env_preserves_slot_number() {
        for slot_number in [None, Some(0), Some(42), Some(u64::MAX)] {
            let header = AnyHeader { slot_number, ..Default::default() };
            let block = block_env_from_header::<BaseEvmTypes>(&header);
            assert_eq!(block.slot_num, U256::from(slot_number.unwrap_or_default()));
        }
    }

    #[test]
    fn block_normalization_uses_source_chain() {
        let header = AnyHeader { number: 500, ..Default::default() };
        let mut block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                BlockTransactions::Full(Vec::new()),
            )
            .into(),
        );
        block.other.insert("l1BlockNumber".to_string(), serde_json::json!("0x64"));

        let mut evm_env =
            env(BlockEnv::<BaseEvmTypes> { number: U256::from(500), ..Default::default() });
        evm_env.set_chain_id(NamedChain::Mainnet as u64);

        apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _>(
            &mut evm_env,
            &block,
            NamedChain::Arbitrum as u64,
            NetworkConfigs::default(),
        );

        assert_eq!(evm_env.chain_id(), NamedChain::Mainnet as u64);
        assert_eq!(evm_env.block.number, U256::from(100));
    }

    #[test]
    fn block_normalization_sets_blob_excess_gas_from_header() {
        // Mainnet block 22_000_000 (Cancun): 22_151_168 excess blob gas prices blobs at 761 wei.
        let header = AnyHeader {
            timestamp: 1_741_410_875,
            excess_blob_gas: Some(22_151_168),
            ..Default::default()
        };
        let block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                BlockTransactions::Full(Vec::new()),
            )
            .into(),
        );
        let mut evm_env = env(BlockEnv::<BaseEvmTypes>::default());
        // The execution chain id can be overridden; the blob schedule follows the source chain.
        evm_env.set_chain_id(1337);

        apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _>(
            &mut evm_env,
            &block,
            NamedChain::Mainnet as u64,
            NetworkConfigs::default(),
        );

        assert_eq!(evm_env.block.blob_basefee, U256::from(761));
    }

    #[test]
    fn block_normalization_keeps_default_blob_env_without_header_field() {
        let header = AnyHeader { excess_blob_gas: None, ..Default::default() };
        let block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                BlockTransactions::Full(Vec::new()),
            )
            .into(),
        );
        let mut evm_env = env(BlockEnv::<BaseEvmTypes>::default());

        apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _>(
            &mut evm_env,
            &block,
            NamedChain::Mainnet as u64,
            NetworkConfigs::default(),
        );

        assert_eq!(evm_env.block.blob_basefee, BlockEnv::<BaseEvmTypes>::default().blob_basefee);
    }

    #[test]
    fn block_normalization_handles_missing_prevrandao_deterministically() {
        for (chain, configs, bypass) in [
            (NamedChain::Moonbeam as u64, NetworkConfigs::default(), true),
            (NamedChain::Gnosis as u64, NetworkConfigs::default(), true),
            (NamedChain::Rsk as u64, NetworkConfigs::default(), true),
            (98_765_432, NetworkConfigs::default(), false),
            (
                98_765_432,
                serde_json::from_str(r#"{"bypass_prevrandao":true,"celo":false}"#).unwrap(),
                true,
            ),
        ] {
            for difficulty in [U256::ZERO, U256::from(42)] {
                let header = AnyHeader { difficulty, mix_hash: None, ..Default::default() };
                let block = AnyRpcBlock::new(
                    Block::new(
                        AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                        BlockTransactions::Full(Vec::new()),
                    )
                    .into(),
                );
                let mut previous = None;
                for _ in 0..2 {
                    let mut evm_env = env(block_env_from_header::<BaseEvmTypes>(block.header()));
                    apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _>(
                        &mut evm_env,
                        &block,
                        chain,
                        configs,
                    );
                    let expected = if bypass { difficulty } else { U256::ZERO };
                    assert_eq!(evm_env.block.prevrandao, expected);
                    assert_eq!(evm_env.block.difficulty, difficulty);
                    if let Some(previous) = &previous {
                        assert_eq!(&evm_env.block, previous);
                    }
                    previous = Some(evm_env.block);
                }
            }
        }
    }

    #[test]
    fn block_normalization_preserves_header_prevrandao() {
        let randao = B256::repeat_byte(0xab);
        let header = AnyHeader { mix_hash: Some(randao), ..Default::default() };
        let block = AnyRpcBlock::new(
            Block::new(
                AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                BlockTransactions::Full(Vec::new()),
            )
            .into(),
        );
        let mut evm_env = env(block_env_from_header::<BaseEvmTypes>(block.header()));
        apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _>(
            &mut evm_env,
            &block,
            NamedChain::Gnosis as u64,
            NetworkConfigs::default(),
        );
        assert_eq!(evm_env.block.prevrandao, U256::from_be_bytes(randao.0));
    }

    #[test]
    fn block_normalization_preserves_mainnet_merge_boundary() {
        let randao = B256::repeat_byte(0xab);
        for number in [15_537_350, 15_537_351, 15_537_393, 15_537_394] {
            let difficulty = if number < 15_537_394 { U256::from(42) } else { U256::ZERO };
            let header =
                AnyHeader { number, difficulty, mix_hash: Some(randao), ..Default::default() };
            let block = AnyRpcBlock::new(
                Block::new(
                    AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                    BlockTransactions::Full(Vec::new()),
                )
                .into(),
            );
            let mut evm_env = env(block_env_from_header::<BaseEvmTypes>(block.header()));
            apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _>(
                &mut evm_env,
                &block,
                NamedChain::Mainnet as u64,
                NetworkConfigs::default(),
            );
            let expected = if difficulty.is_zero() { randao.into() } else { difficulty };
            assert_eq!(evm_env.block.difficulty, expected, "block {number}");
            assert_eq!(evm_env.block.prevrandao, U256::from_be_bytes(randao.0));
        }
    }

    #[test]
    fn block_normalization_uses_difficulty_as_prevrandao() {
        // These chains keep using `difficulty` and return it from `PREVRANDAO`, so a header
        // `mixHash` of zero (or, on Arbitrum, packed L1 metadata) must not reach the block env.
        for (chain, mix_hash) in [
            (NamedChain::BinanceSmartChain, B256::ZERO),
            (NamedChain::Polygon, B256::ZERO),
            (NamedChain::PolygonAmoy, B256::ZERO),
            (NamedChain::Avalanche, B256::ZERO),
            (NamedChain::AvalancheFuji, B256::ZERO),
            (NamedChain::Arbitrum, B256::repeat_byte(0xab)),
            (NamedChain::ArbitrumNova, B256::repeat_byte(0xab)),
            (NamedChain::ArbitrumSepolia, B256::repeat_byte(0xab)),
        ] {
            let header =
                AnyHeader { difficulty: U256::ONE, mix_hash: Some(mix_hash), ..Default::default() };
            let block = AnyRpcBlock::new(
                Block::new(
                    AnyRpcHeader::from_sealed(header.seal(B256::ZERO)),
                    BlockTransactions::Full(Vec::new()),
                )
                .into(),
            );
            let mut evm_env = env(BlockEnv::<BaseEvmTypes> {
                difficulty: U256::ONE,
                prevrandao: mix_hash.into(),
                ..Default::default()
            });

            apply_chain_and_block_specific_env_changes_for_chain::<AnyNetwork, _>(
                &mut evm_env,
                &block,
                chain as u64,
                NetworkConfigs::default(),
            );

            assert_eq!(
                evm_env.block.prevrandao,
                U256::ONE,
                "{chain:?} should expose `difficulty` as `PREVRANDAO`"
            );
        }
    }

    #[test]
    fn tx_replay_env_changes_relax_fee_checks_per_source_chain() {
        let mut evm_env = env(BlockEnv::<BaseEvmTypes>::default());
        evm_env.set_chain_id(NamedChain::Mainnet as u64);
        assert!(evm_env.version.feature(EvmFeatures::PRIORITY_FEE_CHECK));

        apply_chain_specific_tx_replay_env_changes_for_chain(
            &mut evm_env,
            NamedChain::Arbitrum as u64,
        );
        assert_eq!(evm_env.chain_id(), NamedChain::Mainnet as u64);
        assert!(!evm_env.version.feature(EvmFeatures::PRIORITY_FEE_CHECK));

        evm_env.set_feature(EvmFeatures::PRIORITY_FEE_CHECK, true);
        apply_chain_specific_tx_replay_env_changes_for_chain(
            &mut evm_env,
            NamedChain::Mainnet as u64,
        );
        assert!(evm_env.version.feature(EvmFeatures::PRIORITY_FEE_CHECK));
        assert!(evm_env.version.feature(EvmFeatures::BASE_FEE_CHECK));

        apply_chain_specific_tx_replay_env_changes_for_chain(&mut evm_env, NamedChain::Celo as u64);
        assert!(!evm_env.version.feature(EvmFeatures::BASE_FEE_CHECK));
        assert!(evm_env.version.feature(EvmFeatures::PRIORITY_FEE_CHECK));
    }

    #[test]
    fn blob_params_by_spec_id_tracks_latest_known_blob_schedule() {
        assert_eq!(get_blob_params_by_spec_id(SpecId::CANCUN), BlobParams::cancun());
        assert_eq!(get_blob_params_by_spec_id(SpecId::PRAGUE), BlobParams::prague());
        assert_eq!(get_blob_params_by_spec_id(SpecId::OSAKA), BlobParams::osaka());
        assert_eq!(get_blob_params_by_spec_id(SpecId::AMSTERDAM), BlobParams::bpo2());
        assert_eq!(
            get_blob_base_fee_update_fraction_by_spec_id(SpecId::AMSTERDAM),
            BlobParams::bpo2().update_fraction as u64
        );
    }

    #[test]
    fn blob_params_by_explicit_hardfork() {
        for (hardfork, expected) in [
            (EthereumHardfork::Cancun, BlobParams::cancun()),
            (EthereumHardfork::Prague, BlobParams::prague()),
            (EthereumHardfork::Osaka, BlobParams::osaka()),
            (EthereumHardfork::Bpo1, BlobParams::bpo1()),
            (EthereumHardfork::Bpo2, BlobParams::bpo2()),
            (EthereumHardfork::Amsterdam, BlobParams::bpo2()),
        ] {
            assert_eq!(get_blob_params_by_hardfork(hardfork.into()), expected);
        }
    }
}
