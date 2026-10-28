//! Consume fresh Tempo HTTP evidence with the independent upstream Solidity verifiers.

use alloy_json_abi::JsonAbi;
use anvil::{NodeConfig, spawn};
use forge::result::TestStatus;
use foundry_common::fs::read_json_file;
use foundry_compilers::artifacts::{
    ConfigurableContractArtifact, EvmVersion, remappings::Remapping,
};
use foundry_config::{
    SolcReq,
    fs_permissions::{FsPermissions, PathPermission},
};
use foundry_evm::hardfork::TempoHardfork;
use foundry_test_utils::{snapbox::cmd::OutputAssert, util::TestProject};
use rustix::process::{Pid, Signal, kill_process_group};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU64,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{process::Command, time::timeout};

mod evidence;
mod sources;

const SUITE: &str = "test/TempoProof.t.sol:TempoProofTest";
const POSITIVES: [&str; 2] =
    ["test_certified_storage_proof()", "test_rotation_authenticates_next_key_and_storage()"];

#[forgetest]
async fn tempo_proofs(prj: _) {
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
    run_forge(&prj, &["build"]).await;
    let store = read_json_file::<ConfigurableContractArtifact>(
        &prj.artifacts().join("TempoProofVerifier.sol/ProofStore.json"),
    )
    .unwrap();
    let bytecode = store.bytecode.unwrap().into_bytes().unwrap();
    let deployed = store.deployed_bytecode.unwrap().bytecode.unwrap().into_bytes().unwrap();
    let (_api, handle) = spawn(
        NodeConfig::test_tempo()
            .with_no_mining(true)
            .with_hardfork(Some(TempoHardfork::T14.into()))
            .with_tempo_epoch_length(NonZeroU64::new(4).unwrap()),
    )
    .await;
    let proof = timeout(
        Duration::from_secs(60),
        evidence::collect(&handle.http_provider(), bytecode, deployed),
    )
    .await
    .expect("collect fresh Tempo HTTP evidence within 60 seconds");
    prj.create_file("fixtures/proofs.json", &serde_json::to_string_pretty(&proof).unwrap());
    let output = run_forge(&prj, &["test", "--offline", "--json"]).await;
    let suites = serde_json::from_slice::<BTreeMap<String, SoliditySuite>>(&output.stdout).unwrap();
    let artifact = read_json_file::<ConfigurableContractArtifact>(
        &prj.artifacts().join("TempoProof.t.sol/TempoProofTest.json"),
    )
    .unwrap();
    assert!(
        complete_suite(&artifact.abi.unwrap(), &suites),
        "missing, skipped or failed Solidity tests"
    );
}

#[derive(Deserialize)]
struct SoliditySuite {
    test_results: BTreeMap<String, SolidityTest>,
}

#[derive(Deserialize)]
struct SolidityTest {
    status: TestStatus,
}

fn complete_suite(abi: &JsonAbi, suites: &BTreeMap<String, SoliditySuite>) -> bool {
    let expected = abi
        .functions()
        .filter(|function| function.name.starts_with("test_"))
        .map(|function| function.signature())
        .collect::<BTreeSet<_>>();
    suites.len() == 1
        && POSITIVES.iter().all(|name| expected.contains(*name))
        && suites.get(SUITE).is_some_and(|suite| {
            suite.test_results.keys().cloned().collect::<BTreeSet<_>>() == expected
                && suite.test_results.values().all(|test| test.status.is_success())
        })
}

async fn run_forge(prj: &TestProject, arguments: &[&str]) -> Output {
    let mut command = Command::from(prj.forge_bin());
    command.args(arguments).stdout(Stdio::piped()).stderr(Stdio::piped());
    timeout(Duration::from_secs(300), run_command(command))
        .await
        .expect("Forge and its compiler must finish within five minutes")
}

async fn run_command(mut command: Command) -> Output {
    command.process_group(0).kill_on_drop(true);
    let child = command.spawn().unwrap();
    let group = ProcessGroup(Pid::from_raw(child.id().unwrap() as i32).unwrap());
    let output = child.wait_with_output().await.unwrap();
    drop(group);
    OutputAssert::new(output.clone()).success();
    output
}

struct ProcessGroup(Pid);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Terminate compiler descendants on normal completion or cancellation.
        let _ = kill_process_group(self.0, Signal::KILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::{fd::OwnedFd, unix::net::UnixStream};
    use tokio::{io::AsyncReadExt, net::UnixStream as AsyncUnixStream};

    #[test]
    fn rejects_incomplete_solidity_suite() {
        let mut abi = JsonAbi::new();
        for signature in POSITIVES.into_iter().chain(["test_reject_changed_header()"]) {
            abi.functions
                .entry(signature.split('(').next().unwrap().into())
                .or_default()
                .push(alloy_json_abi::Function::parse(signature).unwrap());
        }
        let tests = abi
            .functions()
            .map(|function| (function.signature(), SolidityTest { status: TestStatus::Success }))
            .collect();
        let mut suites = BTreeMap::from([(SUITE.into(), SoliditySuite { test_results: tests })]);
        assert!(complete_suite(&abi, &suites));
        for name in POSITIVES.into_iter().chain(["test_reject_changed_header()"]) {
            let test = suites.get_mut(SUITE).unwrap().test_results.remove(name).unwrap();
            assert!(!complete_suite(&abi, &suites));
            suites.get_mut(SUITE).unwrap().test_results.insert(name.into(), test);
        }
        for status in [TestStatus::Skipped, TestStatus::Failure] {
            suites.get_mut(SUITE).unwrap().test_results.get_mut(POSITIVES[0]).unwrap().status =
                status;
            assert!(!complete_suite(&abi, &suites));
        }
        assert!(!complete_suite(&abi, &BTreeMap::new()));
    }

    #[tokio::test]
    async fn cancelling_command_kills_descendants() {
        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut reader = AsyncUnixStream::from_std(reader).unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 60 & printf ready; wait"])
            .stdout(Stdio::from(OwnedFd::from(writer)));
        let mut running = Box::pin(run_command(command));
        let mut ready = [0; 5];
        tokio::select! {
            output = &mut running => panic!("command ended before cancellation: {output:?}"),
            result = timeout(Duration::from_secs(5), reader.read_exact(&mut ready)) => {
                result.expect("shell must start its descendant").unwrap();
            }
        }
        assert_eq!(&ready, b"ready");
        drop(running);

        // A surviving descendant would keep its inherited stdout open.
        timeout(Duration::from_secs(5), reader.read_to_end(&mut Vec::new()))
            .await
            .expect("compiler descendant survived cancellation")
            .unwrap();
    }
}
