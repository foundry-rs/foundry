//! Independent native Commonware decoding and verification of Anvil's Tempo RPC output.

#![cfg(unix)]

use alloy_consensus::{
    Sealable, Typed2718, proofs::calculate_transaction_root, transaction::TxHashRef,
};
use alloy_primitives::{Address, B256, Bytes, U256, hex, keccak256};
use alloy_provider::Provider;
use anvil::{NodeConfig, NodeHandle, eth::backend::db::EvmDb, spawn};
use commonware_codec::{DecodeExt, Encode};
use commonware_consensus::{
    simplex::{scheme::bls12381_threshold::vrf::Scheme, types::Finalization},
    types::{Epoch, Round, View},
};
use commonware_cryptography::{
    Signer,
    bls12381::primitives::{
        group::Private,
        ops,
        variant::{MinSig, Variant},
    },
    ed25519::{PrivateKey, PublicKey},
    sha256::Digest,
};
use commonware_math::algebra::Random;
use commonware_parallel::Sequential;
use evm2::{
    SpecId,
    evm::{DynDatabase, EmptyDB, precompile::NoPrecompiles},
};
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use reth_evm::{BlockExecutor, BlockExecutorFactory, ConfigureEvm};
use reth_primitives_traits::SealedBlock;
use serde::Deserialize;
use std::num::NonZeroU64;
use tempo_dkg_onchain_artifacts::OnchainDkgOutcome;
use tempo_evm::{
    TempoBlockEnv, TempoEvmConfig, TempoEvmExt, TempoStateAccess, build_tempo_evm,
    tempo_execution_config,
};
use tempo_hardfork::TempoHardfork;
use tempo_precompiles::{
    storage::StorageCtx,
    validator_config::{IValidatorConfig, ValidatorConfig},
    validator_config_v2::{IValidatorConfigV2, ValidatorConfigV2},
};

type NativeCertificate = Finalization<Scheme<PublicKey, MinSig>, Digest>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identity {
    identity: Bytes,
    from_epoch: u64,
}

#[derive(Debug, Deserialize)]
struct CertifiedBlock {
    epoch: u64,
    view: u64,
    digest: B256,
    certificate: String,
    block: tempo_primitives::Block,
}

#[tokio::test(flavor = "multi_thread")]
async fn native_empty_block() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let identity = consensus_identity(&handle).await;
    api.mine_one().await.unwrap();
    let finalization = verify_block(&handle, 1, &identity.identity).await;
    assert_eq!((finalization.epoch, finalization.view), (0, 1));
    assert!(finalization.block.body.transactions.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn native_nonempty_block() {
    let (api, handle) = spawn(NodeConfig::test_tempo().with_no_mining(true)).await;
    let identity = consensus_identity(&handle).await;
    let provider = handle.http_provider();
    let accounts = handle.dev_accounts().collect::<Vec<_>>();
    let ethereum_hash = provider
        .client()
        .request::<_, B256>(
            "eth_sendTransaction",
            (serde_json::json!({
                "from": accounts[0],
                "to": accounts[2],
                "type": "0x2",
                "gas": "0xf4240",
            }),),
        )
        .await
        .unwrap();
    let tempo_hash = provider
        .client()
        .request::<_, B256>(
            "eth_sendTransaction",
            (serde_json::json!({
                "from": accounts[1],
                "type": "0x76",
                "gas": "0x1e8480",
                "calls": [{ "to": accounts[2], "value": "0x0", "input": "0x1234" }],
            }),),
        )
        .await
        .unwrap();
    api.mine_one().await.unwrap();
    let finalization = verify_block(&handle, 1, &identity.identity).await;
    let transactions = &finalization.block.body.transactions;
    let mut transactions =
        transactions.iter().map(|tx| (tx.ty(), *tx.tx_hash())).collect::<Vec<_>>();
    transactions.sort_unstable();
    assert_eq!(transactions, [(2, ethereum_hash), (0x76, tempo_hash)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn native_epoch_boundaries() {
    for hardfork in [TempoHardfork::T3, TempoHardfork::T4] {
        let (api, handle) = spawn(
            NodeConfig::test_tempo()
                .with_no_mining(true)
                .with_hardfork(Some(hardfork.into()))
                .with_tempo_epoch_length(NonZeroU64::new(3)),
        )
        .await;
        let identity = consensus_identity(&handle).await;
        for (height, round) in [(1, (0, 1)), (2, (0, 2)), (3, (1, 1))] {
            api.mine_one().await.unwrap();
            let finalization = verify_block(&handle, height, &identity.identity).await;
            assert_eq!((finalization.epoch, finalization.view), round);
            assert_eq!(finalization.block.header.consensus_context.is_some(), hardfork.is_t4());
            assert_eq!(finalization.block.header.inner.extra_data.is_empty(), height != 2);
        }
    }
}

async fn verify_block(handle: &NodeHandle, height: u64, trusted_identity: &[u8]) -> CertifiedBlock {
    let provider = handle.http_provider();
    let finalization = provider
        .client()
        .request::<_, CertifiedBlock>(
            "consensus_getFinalization",
            (serde_json::json!({ "height": height }),),
        )
        .await
        .unwrap();
    let header = &finalization.block.header;
    let body = &finalization.block.body;
    assert_eq!(header.inner.number, height);
    let block = provider.get_block_by_number(height.into()).await.unwrap().unwrap();
    let raw: Bytes =
        provider.client().request("debug_getRawHeader", (block.header.hash,)).await.unwrap();
    assert_eq!(raw.as_ref(), alloy_rlp::encode(header));
    assert_eq!(keccak256(&raw), finalization.digest);
    assert_eq!(block.header.hash, finalization.digest);
    let identity =
        <MinSig as Variant>::Public::decode(commonware_codec::Copying(trusted_identity)).unwrap();
    let verifier = Scheme::<PublicKey, MinSig>::certificate_verifier(b"TEMPO", identity);
    let bytes = hex::decode(&finalization.certificate).unwrap();
    let certificate =
        NativeCertificate::decode(commonware_codec::Copying(bytes.as_slice())).unwrap();
    assert_eq!(certificate.proposal.round.epoch().get(), finalization.epoch);
    assert_eq!(certificate.proposal.round.view().get(), finalization.view);
    assert_eq!(certificate.proposal.payload.0, finalization.digest.0);
    assert_eq!(calculate_transaction_root(&body.transactions), header.inner.transactions_root);
    assert_eq!(body.calculate_ommers_root(), header.inner.ommers_hash);
    assert_eq!(body.calculate_withdrawals_root(), header.inner.withdrawals_root);
    assert_eq!(certificate.encode().as_ref(), bytes.as_slice());
    if let Some(context) = header.consensus_context {
        assert_eq!(context.epoch, finalization.epoch);
        assert_eq!(context.view, finalization.view);
        assert_eq!(context.parent_view, certificate.proposal.parent.get());
    }
    let mut rng = ChaCha20Rng::from_seed([0; 32]);
    assert!(certificate.verify(&mut rng, &verifier, &Sequential), "valid certificate");
    let wrong_key = ops::compute_public::<MinSig>(&Private::random(&mut rng));
    let wrong_verifier = Scheme::<PublicKey, MinSig>::certificate_verifier(b"TEMPO", wrong_key);
    assert!(!certificate.verify(&mut rng, &wrong_verifier, &Sequential), "wrong identity");

    let mut tampered = certificate.clone();
    tampered.proposal.payload.0[0] ^= 1;
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed digest");
    let mut tampered = certificate.clone();
    tampered.proposal.parent = View::new(certificate.proposal.parent.get() ^ 1);
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed parent view");
    let mut tampered = certificate.clone();
    tampered.proposal.round =
        Round::new(Epoch::new(finalization.epoch ^ 1), certificate.proposal.round.view());
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed epoch");
    let mut tampered = certificate.clone();
    tampered.proposal.round =
        Round::new(certificate.proposal.round.epoch(), View::new(finalization.view + 1));
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed view");

    let mut tampered = certificate.clone();
    let mut signature = certificate.certificate.get().unwrap().clone();
    signature.vote_signature = signature.seed_signature;
    tampered.certificate = signature.into();
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed vote signature");
    let mut tampered = certificate.clone();
    let mut signature = certificate.certificate.get().unwrap().clone();
    signature.seed_signature = signature.vote_signature;
    tampered.certificate = signature.into();
    assert!(!tampered.verify(&mut rng, &verifier, &Sequential), "changed seed signature");
    finalization
}

async fn consensus_identity(handle: &NodeHandle) -> Identity {
    handle.http_provider().client().request("anvil_tempoConsensusIdentity", ()).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn native_rotation_follows_authenticated_announcement() {
    for hardfork in [TempoHardfork::T3, TempoHardfork::T4] {
        let (api, handle) = spawn(
            NodeConfig::test_tempo()
                .with_no_mining(true)
                .with_hardfork(Some(hardfork.into()))
                .with_tempo_epoch_length(NonZeroU64::new(3)),
        )
        .await;
        let provider = handle.http_provider();
        let initial = consensus_identity(&handle).await;
        let scheduled: Identity =
            provider.client().request("anvil_tempoRotateConsensusIdentity", ()).await.unwrap();
        assert_eq!(scheduled.from_epoch, 2);
        assert_ne!(scheduled.identity, initial.identity);
        api.mine_one().await.unwrap();
        let snapshot: U256 = provider.client().request("evm_snapshot", ()).await.unwrap();
        api.mine_one().await.unwrap();

        let preannouncement = verify_block(&handle, 2, &initial.identity).await;
        let ceremony = OnchainDkgOutcome::decode(commonware_codec::Copying(
            preannouncement.block.header.inner.extra_data.as_ref(),
        ))
        .unwrap();
        assert_eq!(ceremony.epoch, 1);
        assert!(ceremony.is_next_full_dkg);
        assert_eq!(ceremony.network_identity().encode().as_ref(), initial.identity.as_ref());
        assert_ne!(ceremony.next_players, *ceremony.output.players());
        api.mine_one().await.unwrap();
        verify_block(&handle, 3, &initial.identity).await;
        api.mine_one().await.unwrap();
        api.mine_one().await.unwrap();

        let boundary = verify_block(&handle, 5, &initial.identity).await;
        let announcement = OnchainDkgOutcome::decode(commonware_codec::Copying(
            boundary.block.header.inner.extra_data.as_ref(),
        ))
        .unwrap();
        assert_eq!(announcement.epoch, scheduled.from_epoch);
        assert!(!announcement.is_next_full_dkg);
        assert_eq!(ceremony.next_players, *announcement.output.players());
        assert_eq!(announcement.next_players, *announcement.output.players());
        let next_key = announcement.network_identity().encode();
        assert_eq!(next_key.as_ref(), scheduled.identity.as_ref());
        assert_eq!(consensus_identity(&handle).await.identity, initial.identity);
        api.mine_one().await.unwrap();
        let next = verify_block(&handle, 6, &next_key).await;
        assert_eq!((next.epoch, next.view), (announcement.epoch, 1));
        assert_eq!(consensus_identity(&handle).await.identity.as_ref(), next_key.as_ref());

        let bytes = hex::decode(&next.certificate).unwrap();
        let certificate =
            NativeCertificate::decode(commonware_codec::Copying(bytes.as_slice())).unwrap();
        let old_key = <MinSig as Variant>::Public::decode(commonware_codec::Copying(
            initial.identity.as_ref(),
        ))
        .unwrap();
        let old_verifier = Scheme::<PublicKey, MinSig>::certificate_verifier(b"TEMPO", old_key);
        assert!(!certificate.verify(
            &mut ChaCha20Rng::from_seed([0; 32]),
            &old_verifier,
            &Sequential,
        ));
        verify_block(&handle, 2, &initial.identity).await;
        verify_block(&handle, 5, &initial.identity).await;

        assert!(provider.client().request::<_, bool>("evm_revert", (snapshot,)).await.unwrap());
        assert_eq!(consensus_identity(&handle).await.identity, initial.identity);
        api.mine_one().await.unwrap();
        let replayed = verify_block(&handle, 2, &initial.identity).await;
        assert_eq!(
            replayed.block.header.inner.extra_data,
            preannouncement.block.header.inner.extra_data,
        );
        for _ in 3..=5 {
            api.mine_one().await.unwrap();
        }
        let replayed = verify_block(&handle, 5, &initial.identity).await;
        assert_eq!(replayed.block.header.inner.extra_data, boundary.block.header.inner.extra_data);
        api.mine_one().await.unwrap();
        verify_block(&handle, 6, &next_key).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_fork_inherits_skipped_views() {
    let config =
        NodeConfig::test_tempo().with_no_mining(true).with_tempo_epoch_length(NonZeroU64::new(4));
    let (source_api, source) = spawn(config.clone()).await;
    source_api.anvil_mine(Some(U256::from(5)), None).await.unwrap();
    let mut state = source_api.serialized_state(false).await.unwrap();
    let block = state.blocks.iter_mut().find(|block| block.header.inner().number == 5).unwrap();
    let mut anchor = block.header.as_tempo().unwrap().clone();
    let context = anchor.consensus_context.as_mut().unwrap();
    context.view = 8896;
    context.parent_view = 8870;
    let mut parent_hash = anchor.hash_slow();
    block.header = anchor.into();
    source_api.anvil_load_state(serde_json::to_vec(&state).unwrap().into()).await.unwrap();

    let (api, handle) = spawn(
        config
            .with_eth_rpc_url(Some(source.http_endpoint()))
            .with_fork_block_number(Some(5u64))
            .with_tempo_consensus_seed(Some(B256::repeat_byte(0x42))),
    )
    .await;
    let identity = consensus_identity(&handle).await;
    for (height, round) in [(6, (1, 8897, 8896)), (7, (1, 8898, 8897)), (8, (2, 1, 0))] {
        api.mine_one().await.unwrap();
        let finalized = verify_block(&handle, height, &identity.identity).await;
        let header = &finalized.block.header;
        let context = header.consensus_context.unwrap();
        assert_eq!((context.epoch, context.view, context.parent_view), round);
        assert_eq!(header.inner.parent_hash, parent_hash);
        parent_hash = finalized.digest;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn native_t15_checks_anvil_boundary_against_parent_state() {
    let factory = TempoEvmConfig::moderato();
    let chain_id = factory.chain_spec().inner.chain.id();
    let (api, _handle) = spawn(
        NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_chain_id(Some(chain_id))
            .with_hardfork(Some(TempoHardfork::T15.into()))
            .with_tempo_epoch_length(NonZeroU64::new(3)),
    )
    .await;
    let identity = api.anvil_tempo_consensus_identity().await.unwrap();

    let owner = Address::repeat_byte(0xAA);
    api.anvil_set_coinbase(Address::ZERO).await.unwrap();
    let mut registry = build_tempo_evm(
        TempoHardfork::T15,
        chain_id,
        TempoBlockEnv::default(),
        EmptyDB::default(),
        NoPrecompiles::default(),
        TempoEvmExt::default(),
    );
    StorageCtx::enter_evm(&mut registry, || {
        let mut legacy = ValidatorConfig::new();
        legacy.initialize(owner)?;
        legacy.add_validator(
            owner,
            IValidatorConfig::addValidatorCall {
                publicKey: identity.proposer,
                newValidatorAddress: Address::repeat_byte(0x11),
                active: true,
                inboundAddress: "192.168.1.1:8000".into(),
                outboundAddress: "192.168.1.1:8000".into(),
            },
        )?;
        let mut config = ValidatorConfigV2::new();
        config.migrate_validator(owner, IValidatorConfigV2::migrateValidatorCall { idx: 0 })?;
        config.initialize_if_migrated(owner)
    })
    .unwrap();
    registry.state_mut().commit_transaction();
    // Install native API-generated storage before mining, rather than deriving it from an outcome.
    for (&address, storage) in &registry.overlay_db().cache.storage {
        for (&slot, &value) in &storage.slots {
            assert!(api.anvil_set_storage_at(address, slot, value.into()).await.unwrap());
        }
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
    let certified = api.backend.tempo_finalization(Some(2)).await.unwrap();
    assert_eq!(certified.block.header.inner.number, 2);
    assert_eq!(certified.block.header.inner.parent_hash, parent_hash);
    let outcome = OnchainDkgOutcome::decode(commonware_codec::Copying(
        certified.block.header.inner.extra_data.as_ref(),
    ))
    .unwrap();
    let expected = config.boundary_outcome(1, outcome.output.clone());
    assert_eq!(outcome, expected);
    let native_pre_execution = |block: &SealedBlock<tempo_primitives::Block>, hardfork| {
        let mut env = factory.evm_env(block.header()).unwrap();
        env.spec = hardfork;
        env.version = *tempo_execution_config(hardfork, chain_id).version();
        env.block.ext.epoch_length = NonZeroU64::new(3).unwrap();
        let context = factory.context_for_block(block).unwrap();
        let evm = BlockExecutorFactory::evm_with_env(
            &factory,
            EvmDb::new(&parent_db, SpecId::OSAKA, None),
            env,
        );
        factory.create_executor(evm, context).apply_pre_execution_changes()
    };
    let block = SealedBlock::new_unhashed(certified.block);
    native_pre_execution(&block, TempoHardfork::T15).unwrap();

    let mut missing = block.clone_block();
    missing.header.inner.extra_data = Bytes::new();
    let missing = SealedBlock::new_unhashed(missing);
    let error = native_pre_execution(&missing, TempoHardfork::T15).unwrap_err();
    assert_eq!(
        error.to_string(),
        "failed decoding boundary block extra data as DKG outcome: Unexpected End-of-Buffer: Not enough bytes remaining to read data",
    );

    let mut wrong_epoch = outcome.clone();
    wrong_epoch.epoch += 1;
    let mut wrong_players = outcome.clone();
    let mut rng = ChaCha20Rng::from_seed([0x42; 32]);
    wrong_players.next_players = [PrivateKey::random(&mut rng).public_key()].try_into().unwrap();
    let mut wrong_flag = outcome;
    wrong_flag.is_next_full_dkg = !wrong_flag.is_next_full_dkg;
    for wrong in [wrong_epoch, wrong_players, wrong_flag] {
        let mut invalid = block.clone_block();
        invalid.header.inner.extra_data = wrong.encode().into();
        let invalid = SealedBlock::new_unhashed(invalid);
        let error = native_pre_execution(&invalid, TempoHardfork::T15).unwrap_err();
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
        native_pre_execution(&invalid, TempoHardfork::T14).unwrap();
    }
}
