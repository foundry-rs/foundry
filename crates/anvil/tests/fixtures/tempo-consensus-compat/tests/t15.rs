//! Validate Anvil boundary announcements against native T15 execution and its parent state.

#![cfg(unix)]

use alloy_primitives::{Address, B256, Bytes, keccak256};
use alloy_provider::Provider;
use anvil::{
    NodeConfig,
    eth::backend::db::{EvmDb, StateDb},
    spawn,
};
use commonware_codec::{DecodeExt, Encode};
use commonware_cryptography::{Signer, ed25519::PrivateKey};
use commonware_math::algebra::Random;
use commonware_utils::ordered;
use evm2::{SpecId, evm::DynDatabase};
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use reth_evm::{BlockExecutionError, BlockExecutor, BlockExecutorFactory, ConfigureEvm};
use reth_primitives_traits::SealedBlock;
use serde::Deserialize;
use std::num::NonZeroU64;
use tempo_dkg_onchain_artifacts::OnchainDkgOutcome;
use tempo_evm::{TempoEvmConfig, TempoStateAccess, tempo_execution_config};
use tempo_hardfork::TempoHardfork;
use tempo_precompiles::{
    storage::{StorageCtx, hashmap::HashMapStorageProvider},
    validator_config_v2::{IValidatorConfigV2, VALIDATOR_NS_ADD, ValidatorConfigV2},
};
use tempo_validator_config::ValidatorConfig;

#[tokio::test(flavor = "multi_thread")]
async fn native_t15_checks_anvil_boundary_against_parent_state() {
    let seed = B256::repeat_byte(0x41);
    let chain_id = TempoEvmConfig::moderato().chain_spec().inner.chain.id();
    let (api, handle) = spawn(
        NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_chain_id(Some(chain_id))
            .with_hardfork(Some(TempoHardfork::T15.into()))
            .with_tempo_consensus_seed(Some(seed))
            .with_tempo_epoch_length(NonZeroU64::new(3)),
    )
    .await;
    let provider = handle.http_provider();
    let identity: Identity =
        provider.client().request("anvil_tempoConsensusIdentity", ()).await.unwrap();

    // The explicit development seed supplies the proof of possession needed to register its key.
    let mut input = seed.to_vec();
    input.extend_from_slice(&0u64.to_be_bytes());
    let mut rng = ChaCha20Rng::from_seed(keccak256(input).0);
    let validator = PrivateKey::random(&mut rng);
    assert_eq!(B256::from_slice(validator.public_key().as_ref()), identity.proposer);
    let owner = Address::repeat_byte(0xAA);
    let validator_address = Address::repeat_byte(0x11);
    let fee_recipient = Address::repeat_byte(0x22);
    api.anvil_set_coinbase(fee_recipient).await.unwrap();
    let (ingress, egress) = ("192.168.1.1:8000", "192.168.1.1");
    let validator_config = ValidatorConfig {
        chain_id,
        validator_address,
        public_key: identity.proposer,
        ingress: ingress.parse().unwrap(),
        egress: egress.parse().unwrap(),
    };
    let signature = validator.sign(
        VALIDATOR_NS_ADD,
        validator_config.add_validator_message_hash(fee_recipient).as_slice(),
    );
    let mut registry = HashMapStorageProvider::new_with_spec(chain_id, TempoHardfork::T15);
    StorageCtx::enter(&mut registry, || {
        let mut config = ValidatorConfigV2::new();
        config.initialize(owner)?;
        config.add_validator(
            owner,
            IValidatorConfigV2::addValidatorCall {
                validatorAddress: validator_address,
                publicKey: identity.proposer,
                ingress: ingress.into(),
                egress: egress.into(),
                feeRecipient: fee_recipient,
                signature: signature.encode().to_vec().into(),
            },
        )?;
        Ok::<_, tempo_precompiles::error::TempoPrecompileError>(())
    })
    .unwrap();
    // Install native API-generated storage before mining, rather than deriving it from an outcome.
    for (address, slot, value) in registry.into_storage() {
        assert!(api.anvil_set_storage_at(address, slot, value.into()).await.unwrap());
    }
    api.mine_one().await.unwrap();
    assert_eq!(api.backend.best_number(), 1);
    let parent_hash = api.backend.best_hash();
    let parent_db = api.backend.get_db().read().await.current_state();
    let config = (&mut EvmDb::new(&parent_db, SpecId::OSAKA, None) as &mut dyn DynDatabase)
        .dkg_ceremony_config(TempoHardfork::T15, 2)
        .unwrap()
        .expect("initialized parent V2 must enable native boundary validation");
    assert_eq!(config.active_validators.len(), 1);
    assert_eq!(config.active_validators[0].publicKey, identity.proposer);
    api.mine_one().await.unwrap();
    let certified: CertifiedBlock = provider
        .client()
        .request("consensus_getFinalization", (serde_json::json!({ "height": 2 }),))
        .await
        .unwrap();
    assert_eq!(certified.block.header.inner.number, 2);
    assert_eq!(certified.block.header.inner.parent_hash, parent_hash);
    let outcome = OnchainDkgOutcome::decode(commonware_codec::Copying(
        certified.block.header.inner.extra_data.as_ref(),
    ))
    .unwrap();
    let expected = config.boundary_outcome(1, outcome.output.clone());
    assert_eq!(outcome, expected);
    let block = SealedBlock::seal_slow(certified.block.clone());
    native_pre_execution(&parent_db, &block, TempoHardfork::T15, chain_id).unwrap();

    let mut missing = certified.block.clone();
    missing.header.inner.extra_data = Bytes::new();
    let missing = SealedBlock::seal_slow(missing);
    let error =
        native_pre_execution(&parent_db, &missing, TempoHardfork::T15, chain_id).unwrap_err();
    assert_eq!(
        error.to_string(),
        "failed decoding boundary block extra data as DKG outcome: Unexpected End-of-Buffer: Not enough bytes remaining to read data",
    );

    let mut wrong_epoch = outcome.clone();
    wrong_epoch.epoch += 1;
    let mut wrong_players = outcome.clone();
    wrong_players.next_players =
        ordered::Set::from_iter_dedup([PrivateKey::random(&mut rng).public_key()]);
    let mut wrong_flag = outcome;
    wrong_flag.is_next_full_dkg = !wrong_flag.is_next_full_dkg;
    for wrong in [wrong_epoch, wrong_players, wrong_flag] {
        let mut invalid = certified.block.clone();
        invalid.header.inner.extra_data = wrong.encode().into();
        let invalid = SealedBlock::seal_slow(invalid);
        let error =
            native_pre_execution(&parent_db, &invalid, TempoHardfork::T15, chain_id).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "boundary block DKG outcome does not match the parent state:                 epoch {} (expected {}), next players {:?} (expected {:?}),                 full DKG {} (expected {})",
                wrong.epoch,
                expected.epoch,
                wrong.next_players(),
                expected.next_players(),
                wrong.is_next_full_dkg,
                expected.is_next_full_dkg,
            ),
        );
        // Before T15, consensus checks these fields; the execution pre-stage does not.
        native_pre_execution(&parent_db, &invalid, TempoHardfork::T14, chain_id).unwrap();
    }
}

#[derive(Debug, Deserialize)]
struct Identity {
    proposer: B256,
}

#[derive(Debug, Deserialize)]
struct CertifiedBlock {
    block: tempo_primitives::Block,
}

fn native_pre_execution(
    parent: &StateDb,
    block: &SealedBlock<tempo_primitives::Block>,
    hardfork: TempoHardfork,
    chain_id: u64,
) -> Result<(), BlockExecutionError> {
    let factory = TempoEvmConfig::moderato();
    let mut env = factory.evm_env(block.header()).unwrap();
    env.spec = hardfork;
    env.version = *tempo_execution_config(hardfork, chain_id).version();
    env.block.ext.epoch_length = NonZeroU64::new(3).unwrap();
    let context = factory.context_for_block(block).unwrap();
    let evm =
        BlockExecutorFactory::evm_with_env(&factory, EvmDb::new(parent, SpecId::OSAKA, None), env);
    factory.create_executor(evm, context).apply_pre_execution_changes()
}
