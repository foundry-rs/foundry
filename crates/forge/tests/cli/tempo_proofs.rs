//! Consume fresh Tempo HTTP evidence with the independent upstream Solidity verifiers.

use anvil::{NodeConfig, spawn};
use foundry_common::fs::read_json_file;
use foundry_compilers::artifacts::{
    ConfigurableContractArtifact, EvmVersion, remappings::Remapping,
};
use foundry_config::{
    SolcReq,
    fs_permissions::{FsPermissions, PathPermission},
};
use foundry_evm::hardfork::TempoHardfork;
use foundry_test_utils::str;
use std::{num::NonZeroU64, time::Duration};
use tokio::time::timeout;

mod evidence;
mod sources;

#[forgetest]
async fn tempo_proofs_fork(prj: _, cmd: _) {
    sources::prepare(&prj).await;
    prj.insert_vm();
    prj.add_raw_source(
        "TempoProofVerifier.sol",
        include_str!("../../../../testdata/fixtures/TempoProofVerifier.sol"),
    );
    prj.add_raw_test(
        "TempoProof.t.sol",
        include_str!("../../../../testdata/fixtures/TempoProof.t.sol"),
    );
    prj.update_config(|config| {
        config.solc = Some(SolcReq::Version("0.8.30".parse().unwrap()));
        config.evm_version = EvmVersion::Prague;
        config.optimizer = Some(true);
        config.via_ir = true;
        config.fs_permissions = FsPermissions::new([PathPermission::read("fixtures")]);
        config.remappings =
            ["commonware/=lib/commonware/src/", "src/libraries/=lib/optimism/src/libraries/"]
                .map(|remapping| remapping.parse::<Remapping>().unwrap().into())
                .into();
    });
    cmd.args(["build"]).assert_success();
    let store = read_json_file::<ConfigurableContractArtifact>(
        &prj.artifacts().join("TempoProofVerifier.sol/ProofStore.json"),
    )
    .unwrap();
    let bytecode = store.bytecode.unwrap().into_bytes().unwrap();
    let deployed = store.deployed_bytecode.unwrap().bytecode.unwrap().into_bytes().unwrap();
    let config = NodeConfig::test_tempo()
        .with_no_mining(true)
        .with_hardfork(Some(TempoHardfork::T14.into()))
        .with_tempo_epoch_length(NonZeroU64::new(4).unwrap());
    let (_api, handle) = spawn(config.clone()).await;
    let mut proof = timeout(
        Duration::from_secs(60),
        evidence::collect(&handle.http_provider(), bytecode.clone(), deployed.clone()),
    )
    .await
    .expect("collect fresh Tempo HTTP evidence within 60 seconds");
    proof["fork"] =
        timeout(Duration::from_secs(60), evidence::collect_fork(config, bytecode, deployed))
            .await
            .expect("collect local two-node fork evidence within 60 seconds");
    prj.create_file("fixtures/proofs.json", &serde_json::to_string_pretty(&proof).unwrap());
    cmd.forge_fuse().args(["test", "--offline"]).assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 29 tests for test/TempoProof.t.sol:TempoProofTest
[PASS] test_certified_storage_proof() ([GAS])
[PASS] test_fork_historical_certified_storage_proof() ([GAS])
[PASS] test_fork_imported_account_certified_storage_proof() ([GAS])
[PASS] test_fork_reject_mixed_account_proof() ([GAS])
[PASS] test_fork_reject_mixed_storage_proof() ([GAS])
[PASS] test_fork_reject_upstream_key() ([GAS])
[PASS] test_fork_updated_certified_storage_proof() ([GAS])
[PASS] test_reject_changed_account_proof() ([GAS])
[PASS] test_reject_changed_epoch() ([GAS])
[PASS] test_reject_changed_header() ([GAS])
[PASS] test_reject_changed_parent() ([GAS])
[PASS] test_reject_changed_seed() ([GAS])
[PASS] test_reject_changed_storage_proof() ([GAS])
[PASS] test_reject_changed_view() ([GAS])
[PASS] test_reject_changed_vote() ([GAS])
[PASS] test_reject_nonfinalization() ([GAS])
[PASS] test_reject_wrong_account() ([GAS])
[PASS] test_reject_wrong_height() ([GAS])
[PASS] test_reject_wrong_slot() ([GAS])
[PASS] test_reject_wrong_value() ([GAS])
[PASS] test_rotation_authenticates_next_key_and_storage() ([GAS])
[PASS] test_rotation_rejects_changed_successor_epoch() ([GAS])
[PASS] test_rotation_rejects_changed_successor_proof() ([GAS])
[PASS] test_rotation_rejects_malformed_next_key() ([GAS])
[PASS] test_rotation_rejects_noncanonical_next_key() ([GAS])
[PASS] test_rotation_rejects_old_key() ([GAS])
[PASS] test_rotation_rejects_unannounced_key() ([GAS])
[PASS] test_rotation_rejects_unauthenticated_boundary() ([GAS])
[PASS] test_rotation_rejects_wrong_epoch() ([GAS])
Suite result: ok. 29 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 29 tests passed, 0 failed, 0 skipped (29 total tests)

"#]]);
}
