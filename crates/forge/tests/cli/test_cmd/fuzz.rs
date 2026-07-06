use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{U256, hex};
use foundry_config::fs_permissions::PathPermission;
use foundry_evm::fuzz::BaseCounterExample;
use foundry_test_utils::{TestCommand, forgetest_init, str};
use regex::Regex;
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

const DEFAULT_SENDER: &str = "0x0000000000000000000000000000000000000001";
const DEFAULT_TEST_TARGET: &str = "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496";

fn find_first_json(root: &std::path::Path) -> std::path::PathBuf {
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|extension| extension == "json") {
                return path;
            }
        }
    }
    panic!("no json corpus entry under {}", root.display());
}

fn artifact_abi(root: &Path, artifact: &str) -> JsonAbi {
    let artifact = std::fs::read_to_string(root.join(artifact)).unwrap();
    let artifact: Value = serde_json::from_str(&artifact).unwrap();
    serde_json::from_value(artifact["abi"].clone()).unwrap()
}

fn calldata_for(abi: &JsonAbi, function_name: &str, arg: u64) -> String {
    let function = abi.functions().find(|function| function.name == function_name).unwrap();
    format!("0x{}{:064x}", hex::encode(function.selector()), arg)
}

fn calldata_for_args(abi: &JsonAbi, function_name: &str, args: &[DynSolValue]) -> String {
    let function = abi.functions().find(|function| function.name == function_name).unwrap();
    format!("0x{}", hex::encode(function.abi_encode_input(args).unwrap()))
}

fn output_calldata_args(
    root: &Path,
    output: &str,
    abi: &JsonAbi,
    function_name: &str,
) -> Vec<DynSolValue> {
    let output: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join(output)).unwrap()).unwrap();
    let calldata = output[0]["calldata"].as_str().unwrap().trim_start_matches("0x");
    let calldata = hex::decode(calldata).unwrap();
    let function = abi.functions().find(|function| function.name == function_name).unwrap();
    function.abi_decode_input(&calldata[4..]).unwrap()
}

fn corpus_entry(calldata: &str) -> String {
    corpus_sequence_entry(&[calldata])
}

fn corpus_sequence_entry(calldatas: &[&str]) -> String {
    let entries = calldatas
        .iter()
        .map(|calldata| {
            format!(
                r#"{{
  "sender":"{DEFAULT_SENDER}",
  "target":"{DEFAULT_TEST_TARGET}",
  "calldata":"{calldata}",
  "value":"0x0"
}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"[
{entries}
]"#
    )
}

fn write_corpus_entry(corpus: &Path, name: &str, calldata: &str) {
    std::fs::write(corpus.join(name), corpus_entry(calldata)).unwrap();
}

fn write_corpus_sequence_entry(corpus: &Path, name: &str, calldatas: &[&str]) {
    std::fs::write(corpus.join(name), corpus_sequence_entry(calldatas)).unwrap();
}

fn has_regular_file(root: &Path) -> bool {
    root.exists()
        && std::fs::read_dir(root).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path.is_file() || (path.is_dir() && has_regular_file(&path))
        })
}

fn regular_file_count(root: &Path) -> usize {
    if !root.exists() {
        return 0;
    }
    std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| if path.is_dir() { regular_file_count(&path) } else { 1 })
        .sum()
}

fn showmap_edge_ids(root: &Path) -> BTreeSet<String> {
    fn visit(path: &Path, out: &mut BTreeSet<String>) {
        if path.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                visit(&entry.unwrap().path(), out);
            }
            return;
        }
        if path.extension().is_none_or(|extension| extension != "txt") {
            return;
        }
        let body = std::fs::read_to_string(path).unwrap();
        for line in body.lines() {
            let (id, _) = line.split_once(':').unwrap_or_else(|| panic!("malformed {line}"));
            out.insert(id.to_string());
        }
    }

    let mut edges = BTreeSet::new();
    visit(root, &mut edges);
    edges
}

forgetest_init!(test_can_scrape_bytecode, |prj, cmd| {
    prj.update_config(|config| config.optimizer = Some(true));
    prj.add_source(
        "FuzzerDict.sol",
        r#"
// https://github.com/foundry-rs/foundry/issues/1168
contract FuzzerDict {
    // Immutables should get added to the dictionary.
    address public immutable immutableOwner;
    // Regular storage variables should also get added to the dictionary.
    address public storageOwner;

    constructor(address _immutableOwner, address _storageOwner) {
        immutableOwner = _immutableOwner;
        storageOwner = _storageOwner;
    }
}
   "#,
    );

    prj.add_test(
        "FuzzerDictTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/FuzzerDict.sol";

contract FuzzerDictTest is Test {
    FuzzerDict fuzzerDict;

    function setUp() public {
        fuzzerDict = new FuzzerDict(address(100), address(200));
    }

    /// forge-config: default.fuzz.runs = 2000
    function testImmutableOwner(address who) public {
        assertTrue(who != fuzzerDict.immutableOwner());
    }

    /// forge-config: default.fuzz.runs = 2000
    function testStorageOwner(address who) public {
        assertTrue(who != fuzzerDict.storageOwner());
    }
}
   "#,
    );

    // Test that immutable address is used as fuzzed input, causing test to fail.
    cmd.args(["test", "--fuzz-seed", "119", "--mt", "testImmutableOwner"]).assert_failure();
    // Test that storage address is used as fuzzed input, causing test to fail.
    cmd.forge_fuse()
        .args(["test", "--fuzz-seed", "119", "--mt", "testStorageOwner"])
        .assert_failure();
});

forgetest_init!(forge_fuzz_run_skips_unit_tests, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzRun.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzRunTest is Test {
    function test_unit() public pure {}

    /// forge-config: default.fuzz.runs = 2
    function testFuzz_value(uint256 value) public pure {
        value;
    }
}
   "#,
    );

    cmd.args(["fuzz", "run", "--mc", "ForgeFuzzRunTest"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/ForgeFuzzRun.t.sol:ForgeFuzzRunTest
[PASS] testFuzz_value(uint256) (runs: 2, [AVG_GAS])
[SKIP: not runnable in fuzz mode] test_unit() ([GAS])
Suite result: ok. 1 passed; 0 failed; 1 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 1 skipped (2 total tests)

"#]]);
});

forgetest_init!(does_not_evaluate_unused_fuzz_fixtures_for_unit_test_filter, |prj, cmd| {
    let marker = prj.root().join("fixture-called.txt");
    prj.update_config(|config| config.fs_permissions.add(PathPermission::write(prj.root())));
    prj.add_test(
        "UnusedFuzzFixtures.t.sol",
        r#"
interface Vm {
    function projectRoot() external view returns (string memory path);
    function writeFile(string calldata path, string calldata data) external;
}

contract UnusedFuzzFixturesTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function fixtureAmount() public returns (uint256[] memory values) {
        vm.writeFile(string.concat(vm.projectRoot(), "/fixture-called.txt"), "called");
        values = new uint256[](1);
        values[0] = 1;
    }

    function testUnit() public pure {}

    /// forge-config: default.fuzz.runs = 1
    function testFuzzUsesFixture(uint256 amount) public pure {
        amount;
    }
}
    "#,
    );

    cmd.args(["test", "--match-test", "testUnit", "-q"]).assert_success();
    assert!(!marker.exists(), "unit-only run evaluated fuzz fixture");

    cmd.forge_fuse();
    cmd.args(["test", "--match-test", "testFuzzUsesFixture", "-q"]).assert_success();
    assert!(marker.exists(), "fuzz run did not evaluate fuzz fixture");
});

forgetest_init!(forge_fuzz_skips_unit_only_failing_setup, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzUnitOnly.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzUnitOnlyTest is Test {
    function setUp() public pure {
        revert("setUp should not run");
    }

    function test_unit() public pure {}
}
   "#,
    );

    let run =
        cmd.forge_fuse().args(["fuzz", "run", "--mc", "ForgeFuzzUnitOnlyTest"]).assert_success();
    let stdout = String::from_utf8(run.get_output().stdout.clone()).unwrap();
    assert!(!stdout.contains("setUp should not run"), "{stdout}");

    let replay =
        cmd.forge_fuse().args(["fuzz", "replay", "--mc", "ForgeFuzzUnitOnlyTest"]).assert_success();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(!stdout.contains("setUp should not run"), "{stdout}");
});

forgetest_init!(forge_fuzz_replay_reports_missing_corpus, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzReplay.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzReplayTest is Test {
    function test_unit() public pure {}

    function testFuzz_value(uint256 value) public pure {
        value;
    }
}
   "#,
    );

    cmd.args(["fuzz", "replay", "--mc", "ForgeFuzzReplayTest"]).assert_success().stdout_eq(str![[
        r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/ForgeFuzzReplay.t.sol:ForgeFuzzReplayTest
[SKIP: no persisted fuzz failure found at cache/fuzz/failures/ForgeFuzzReplayTest/testFuzz_value] testFuzz_value(uint256) (runs: 0, [AVG_GAS])
[SKIP: not runnable in replay mode] test_unit() ([GAS])
Suite result: ok. 0 passed; 0 failed; 2 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 0 failed, 2 skipped (2 total tests)

"#
    ]]);
});

// `forge fuzz replay` (without `--corpus-dir`) must not start a fresh invariant
// campaign when there is no persisted failure to replay; it should skip instead.
forgetest_init!(forge_fuzz_replay_invariant_skips_without_persisted_failure, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzReplayInvariant.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzReplayInvariantTest is Test {
    uint256 total;

    function setUp() public {
        targetContract(address(this));
    }

    function doThing(uint256 x) public {
        total += x;
    }

    function invariant_holds() public view {
        assertGe(total, 0);
    }
}
   "#,
    );

    cmd.args(["fuzz", "replay", "--mc", "ForgeFuzzReplayInvariantTest"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/ForgeFuzzReplayInvariant.t.sol:ForgeFuzzReplayInvariantTest
[SKIP: no persisted invariant failure reproduced for invariant_holds] invariant_holds() ([GAS])
Suite result: ok. 0 passed; 0 failed; 1 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 0 failed, 1 skipped (1 total tests)

"#]]);
});

forgetest_init!(forge_fuzz_replay_replays_persisted_fuzz_failure, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.runs = 32;
        config.fuzz.seed = Some(U256::from(100u32));
    });
    prj.add_test(
        "ForgeFuzzReplayFailure.t.sol",
        r#"
contract ForgeFuzzReplayFailureTest {
    function test_unit() public pure {}

    function testFuzz_reverts(uint256 value) public pure {
        require(value > 200);
    }
}
   "#,
    );

    cmd.args(["fuzz", "run", "--mc", "ForgeFuzzReplayFailureTest", "-q"]).assert_failure();

    let replay = cmd
        .forge_fuse()
        .args(["fuzz", "replay", "--mc", "ForgeFuzzReplayFailureTest", "-vvv"])
        .assert_failure();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("[FAIL: EvmError: Revert; counterexample: calldata=0x")
            && stdout.contains("args=[200]] testFuzz_reverts(uint256) (runs: 0,"),
        "{stdout}"
    );
    assert!(stdout.contains("ForgeFuzzReplayFailureTest::testFuzz_reverts(200)"), "{stdout}");
    assert!(stdout.contains("[SKIP: not runnable in replay mode] test_unit()"), "{stdout}");
});

forgetest_init!(forge_fuzz_show_marks_selector_ambiguous_contracts, |prj, cmd| {
    prj.add_source(
        "SelectorTwins.sol",
        r#"
contract Alpha {
    function collide(uint256 value) external pure returns (uint256) {
        return value;
    }
}

contract Beta {
    uint256 public stored;

    function collide(uint256 value) external view {
        require(stored != value);
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let alpha_abi = artifact_abi(prj.root(), "out/SelectorTwins.sol/Alpha.json");
    let calldata = calldata_for(&alpha_abi, "collide", 42);
    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-00000000be7a-1.json", &calldata);

    let show = cmd.forge_fuse().args(["fuzz", "show", "corpus"]).assert_success();
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("  0: collide(42) "), "{stdout}");
    assert!(stdout.contains(" ambiguous=[Alpha,Beta]"), "{stdout}");
    assert!(!stdout.contains("Alpha.collide(42)"), "{stdout}");
    assert!(!stdout.contains("Beta.collide(42)"), "{stdout}");

    let json =
        cmd.forge_fuse().args(["fuzz", "show", "corpus", "--format", "json"]).assert_success();
    let stdout = String::from_utf8(json.get_output().stdout.clone()).unwrap();
    let entries: Value = serde_json::from_str(&stdout).unwrap();
    let decoded = &entries[0]["sequence"][0]["decoded"];
    assert!(decoded.get("contract").is_none(), "{decoded}");
    assert_eq!(decoded["signature"], "collide(uint256)");
    assert_eq!(decoded["call"], "collide(42)");
    assert_eq!(decoded["ambiguous_contracts"], serde_json::json!(["Alpha", "Beta"]));
});

forgetest_init!(forge_fuzz_replay_does_not_fuzz_after_assume_reject, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.runs = 32;
        config.fuzz.seed = Some(U256::from(100u32));
    });
    prj.add_test(
        "ForgeFuzzReplayAssumeReject.t.sol",
        r#"
contract ForgeFuzzReplayAssumeRejectTest {
    function testFuzz_reverts(uint256 value) public pure {
        require(value > 200);
    }
}
   "#,
    );

    cmd.args(["fuzz", "run", "--mc", "ForgeFuzzReplayAssumeRejectTest", "-q"]).assert_failure();

    prj.add_test(
        "ForgeFuzzReplayAssumeReject.t.sol",
        r#"
interface Vm {
    function assume(bool) external;
}

contract ForgeFuzzReplayAssumeRejectTest {
    Vm internal constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function testFuzz_reverts(uint256 value) public {
        vm.assume(value != 200);
        require(false, "fresh unrelated failure");
    }
}
   "#,
    );

    cmd.forge_fuse()
        .args(["fuzz", "replay", "--mc", "ForgeFuzzReplayAssumeRejectTest"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/ForgeFuzzReplayAssumeReject.t.sol:ForgeFuzzReplayAssumeRejectTest
[SKIP: persisted fuzz failure rejected by `vm.assume`] testFuzz_reverts(uint256) (runs: 0, [AVG_GAS])
Suite result: ok. 0 passed; 0 failed; 1 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 0 failed, 1 skipped (1 total tests)

"#]]);
});

forgetest_init!(forge_fuzz_replay_treats_persisted_skip_as_skip, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.runs = 32;
        config.fuzz.seed = Some(U256::from(100u32));
    });
    prj.add_test(
        "ForgeFuzzReplaySkip.t.sol",
        r#"
contract ForgeFuzzReplaySkipTest {
    function testFuzz_reverts(uint256 value) public pure {
        require(value > 200);
    }
}
   "#,
    );

    cmd.args(["fuzz", "run", "--mc", "ForgeFuzzReplaySkipTest", "-q"]).assert_failure();

    prj.add_test(
        "ForgeFuzzReplaySkip.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzReplaySkipTest is Test {
    function testFuzz_reverts(uint256 value) public {
        value;
        vm.skip(true, "disabled");
    }
}
   "#,
    );

    let replay = cmd
        .forge_fuse()
        .args(["fuzz", "replay", "--mc", "ForgeFuzzReplaySkipTest"])
        .assert_success();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[SKIP: disabled] testFuzz_reverts(uint256)"), "{stdout}");
    assert!(!stdout.contains("[FAIL"), "{stdout}");
});

forgetest_init!(forge_fuzz_replay_does_not_treat_user_skip_payload_as_skip, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.runs = 32;
        config.fuzz.seed = Some(U256::from(100u32));
    });
    prj.add_test(
        "ForgeFuzzReplayUserSkipPayload.t.sol",
        r#"
contract ForgeFuzzReplayUserSkipPayloadTest {
    function testFuzz_reverts(uint256 value) public pure {
        require(value > 200);
    }
}
   "#,
    );

    cmd.args(["fuzz", "run", "--mc", "ForgeFuzzReplayUserSkipPayloadTest", "-q"]).assert_failure();

    prj.add_test(
        "ForgeFuzzReplayUserSkipPayload.t.sol",
        r#"
contract ForgeFuzzReplayUserSkipPayloadTest {
    function testFuzz_reverts(uint256 value) public pure {
        value;
        bytes memory reason = bytes("FOUNDRY::SKIPnot cheatcode");
        assembly {
            revert(add(reason, 32), mload(reason))
        }
    }
}
   "#,
    );

    let replay = cmd
        .forge_fuse()
        .args(["fuzz", "replay", "--mc", "ForgeFuzzReplayUserSkipPayloadTest"])
        .assert_failure();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[FAIL:"), "{stdout}");
    assert!(!stdout.contains("[SKIP: not cheatcode]"), "{stdout}");
});

forgetest_init!(forge_fuzz_junit_output_stays_xml_only_on_failure, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.runs = 1;
        config.fuzz.seed = Some(U256::from(100u32));
    });
    prj.add_test(
        "ForgeFuzzJunitFailure.t.sol",
        r#"
contract ForgeFuzzJunitFailureTest {
    function testFuzz_reverts(uint256 value) public pure {
        value;
        require(false);
    }
}
   "#,
    );

    let run = cmd
        .forge_fuse()
        .args(["fuzz", "run", "--junit", "--mc", "ForgeFuzzJunitFailureTest"])
        .assert_failure();
    let stdout = String::from_utf8(run.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("<testsuites"), "{stdout}");
    assert!(!stdout.contains("Failing tests:"), "{stdout}");

    let replay = cmd
        .forge_fuse()
        .args(["fuzz", "replay", "--junit", "--mc", "ForgeFuzzJunitFailureTest"])
        .assert_failure();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("<testsuites"), "{stdout}");
    assert!(!stdout.contains("Failing tests:"), "{stdout}");
});

forgetest_init!(forge_fuzz_rejects_watch, |prj, cmd| {
    let run = cmd.forge_fuse().args(["fuzz", "run", "--watch"]).assert_failure();
    let stderr = String::from_utf8(run.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("`--watch` is not supported for `forge fuzz run`"), "{stderr}");

    let replay = cmd.forge_fuse().args(["fuzz", "replay", "--watch"]).assert_failure();
    let stderr = String::from_utf8(replay.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("`--watch` is not supported for `forge fuzz replay`"), "{stderr}");
});

forgetest_init!(forge_fuzz_list_only_shows_runnable_tests, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzList.t.sol",
        r#"
contract ForgeFuzzListTest {
    function test_unit() public {}
    function testFuzz_value(uint256 value) public pure {
        value;
    }
    function invariant_ok() public pure {}
    function table_row() public pure {}
}
   "#,
    );

    cmd.args(["fuzz", "run", "--list", "--mc", "ForgeFuzzListTest"]).assert_success().stdout_eq(
        str![[r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
test/ForgeFuzzList.t.sol
  ForgeFuzzListTest
    invariant_ok
    testFuzz_value


"#]],
    );
});

forgetest_init!(forge_showmap_skips_symbolic_tests, |prj, cmd| {
    prj.add_test(
        "ForgeShowmapSymbolic.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeShowmapSymbolicTest is Test {
    function check_symbolic(uint256 value) public pure {
        value;
    }
}
   "#,
    );

    let assert = cmd
        .args([
            "test",
            "--symbolic",
            "--showmap-out",
            "showmap",
            "--mc",
            "ForgeShowmapSymbolicTest",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("[SKIP: not runnable in showmap mode] check_symbolic(uint256)"),
        "{stdout}"
    );
});

forgetest_init!(forge_fuzz_show_corpus_files, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzShowTarget.t.sol",
        r#"
contract ForgeFuzzShowTargetTest {
    function testFuzz_setNumber(uint256 value) public pure {
        value;
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let calldata = "0x938872f7000000000000000000000000000000000000000000000000000000000000002a";
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", calldata);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000002-2.json", calldata);

    cmd.forge_fuse()
        .args(["fuzz", "show", "corpus"])
        .assert_success()
        .stdout_eq(str![[r#"
corpus/00000000-0000-0000-0000-000000000001-1.json (1 txs)
  0: ForgeFuzzShowTargetTest.testFuzz_setNumber(42) sender=0x0000000000000000000000000000000000000001 target=0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496 value=0
corpus/00000000-0000-0000-0000-000000000002-2.json (1 txs)
  0: ForgeFuzzShowTargetTest.testFuzz_setNumber(42) sender=0x0000000000000000000000000000000000000001 target=0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496 value=0

"#]]);

    let replay = cmd
        .forge_fuse()
        .args(["fuzz", "replay", "--mc", "ForgeFuzzShowTargetTest", "--corpus-dir", "corpus"])
        .assert_success();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[PASS] testFuzz_setNumber(uint256) (replay: 2 entries"), "{stdout}");
});

forgetest_init!(forge_fuzz_cmin_keeps_coverage_adding_entries, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminTarget.t.sol",
        r#"
contract ForgeFuzzCminTargetTest {
    uint256 value;

    function testFuzz_branch(uint256 input) public {
        if (input == 1) {
            value = 1;
        }
        if (input == 2) {
            value = 2;
        }
        if (input == 3) {
            revert("boom");
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi =
        artifact_abi(prj.root(), "out/ForgeFuzzCminTarget.t.sol/ForgeFuzzCminTargetTest.json");
    let one = calldata_for(&abi, "testFuzz_branch", 1);
    let two = calldata_for(&abi, "testFuzz_branch", 2);
    let three = calldata_for(&abi, "testFuzz_branch", 3);
    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &one);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000002-2.json", &one);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000003-3.json", &two);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000004-4.json", &three);
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000005-5.json"), "[]").unwrap();

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminTargetTest",
            "--mt",
            "testFuzz_branch",
            "corpus",
            "--corpus-out",
            "min-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stdout.contains("minimized corpus: kept 2/5 entries in min-corpus"), "{stdout}");
    assert!(stderr.contains("skipped 2 entries or txs"), "{stderr}");
    assert_eq!(regular_file_count(&prj.root().join("min-corpus")), 2);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminTargetTest",
            "--mt",
            "testFuzz_branch",
            "corpus",
            "--corpus-out",
            "min-corpus",
        ])
        .assert_failure();

    let preserve_corpus = prj.root().join("preserve-corpus");
    std::fs::create_dir_all(&preserve_corpus).unwrap();
    write_corpus_entry(&preserve_corpus, "00000000-0000-0000-0000-000000000001-1.json", &one);
    write_corpus_entry(&preserve_corpus, "00000000-0000-0000-0000-000000000002-2.json", &one);
    write_corpus_entry(&preserve_corpus, "00000000-0000-0000-0000-000000000003-3.json", &two);
    cmd.forge_fuse()
        .args([
            "test",
            "--mc",
            "ForgeFuzzCminTargetTest",
            "--mt",
            "testFuzz_branch",
            "--showmap-out",
            "showmap-before-cmin",
            "--showmap-corpus-dir",
            "preserve-corpus",
            "--showmap-trial",
            "t",
        ])
        .assert_success();
    cmd.forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminTargetTest",
            "--mt",
            "testFuzz_branch",
            "preserve-corpus",
            "--corpus-out",
            "min-preserve-corpus",
        ])
        .assert_success();
    cmd.forge_fuse()
        .args([
            "test",
            "--mc",
            "ForgeFuzzCminTargetTest",
            "--mt",
            "testFuzz_branch",
            "--showmap-out",
            "showmap-after-cmin",
            "--showmap-corpus-dir",
            "min-preserve-corpus",
            "--showmap-trial",
            "t",
        ])
        .assert_success();
    assert_eq!(
        showmap_edge_ids(&prj.root().join("showmap-before-cmin")),
        showmap_edge_ids(&prj.root().join("showmap-after-cmin"))
    );
});

forgetest_init!(forge_fuzz_cmin_keeps_hit_count_bucket_increases, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminBucketTarget.t.sol",
        r#"
contract ForgeFuzzCminBucketTargetTest {
    uint256 value;

    function testFuzz_reps(uint256 input) public {
        uint256 n = input % 5;
        for (uint256 i = 0; i < n; i++) {
            value = value + i + 1;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzCminBucketTarget.t.sol/ForgeFuzzCminBucketTargetTest.json",
    );
    let one = calldata_for(&abi, "testFuzz_reps", 1);
    let four = calldata_for(&abi, "testFuzz_reps", 4);
    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &one);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000002-2.json", &four);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminBucketTargetTest",
            "--mt",
            "testFuzz_reps",
            "corpus",
            "--corpus-out",
            "min-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("minimized corpus: kept 2/2 entries in min-corpus"), "{stdout}");
    assert_eq!(regular_file_count(&prj.root().join("min-corpus")), 2);
});

forgetest_init!(forge_fuzz_cmin_handles_multiple_matched_targets, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminMultiTarget.t.sol",
        r#"
contract ForgeFuzzCminMultiTargetTest {
    uint256 left;
    uint256 right;

    function testFuzz_left(uint256 input) public {
        if (input == 1) {
            left = 1;
        }
    }

    function testFuzz_right(uint256 input) public {
        if (input == 2) {
            right = 2;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzCminMultiTarget.t.sol/ForgeFuzzCminMultiTargetTest.json",
    );
    let left = calldata_for(&abi, "testFuzz_left", 1);
    let right = calldata_for(&abi, "testFuzz_right", 2);
    let corpus = prj.root().join("multi-target-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &left);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000002-2.json", &right);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminMultiTargetTest",
            "multi-target-corpus",
            "--corpus-out",
            "min-multi-target-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("minimized corpus: kept 2/2 entries in min-multi-target-corpus"),
        "{stdout}"
    );
    assert_eq!(regular_file_count(&prj.root().join("min-multi-target-corpus")), 2);
});

forgetest_init!(forge_fuzz_cmin_namespaces_coverage_by_matched_target, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminNamespacedTargets.t.sol",
        r#"
contract ForgeFuzzCminNamespacedAlphaTest {
    uint256 value;

    function testFuzz_shared(uint256 input) public {
        if (input == 1) {
            value = 1;
        }
    }
}

contract ForgeFuzzCminNamespacedBetaTest {
    uint256 value;

    function testFuzz_shared(uint256 input) public {
        if (input == 2) {
            value = 2;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzCminNamespacedTargets.t.sol/ForgeFuzzCminNamespacedAlphaTest.json",
    );
    let one = calldata_for(&abi, "testFuzz_shared", 1);
    let two = calldata_for(&abi, "testFuzz_shared", 2);
    let corpus = prj.root().join("namespaced-target-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &one);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000002-2.json", &two);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminNamespaced(Alpha|Beta)Test",
            "--mt",
            "testFuzz_shared",
            "namespaced-target-corpus",
            "--corpus-out",
            "min-namespaced-target-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("minimized corpus: kept 2/2 entries in min-namespaced-target-corpus"),
        "{stdout}"
    );
    assert_eq!(regular_file_count(&prj.root().join("min-namespaced-target-corpus")), 2);
});

forgetest_init!(forge_fuzz_cmin_keeps_entry_when_one_target_fails, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminTargetFailure.t.sol",
        r#"
contract ForgeFuzzCminTargetFailureAlphaTest {
    uint256 value;

    function testFuzz_shared(uint256 input) public {
        if (input == 1) {
            value = 1;
        }
    }
}

contract ForgeFuzzCminTargetFailureBetaTest {
    function testFuzz_shared(uint256 input) public pure {
        if (input == 1) {
            revert("beta rejects");
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzCminTargetFailure.t.sol/ForgeFuzzCminTargetFailureAlphaTest.json",
    );
    let one = calldata_for(&abi, "testFuzz_shared", 1);
    let corpus = prj.root().join("target-failure-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &one);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminTargetFailure(Alpha|Beta)Test",
            "--mt",
            "testFuzz_shared",
            "target-failure-corpus",
            "--corpus-out",
            "min-target-failure-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("minimized corpus: kept 1/1 entries in min-target-failure-corpus"),
        "{stdout}"
    );
    assert_eq!(regular_file_count(&prj.root().join("min-target-failure-corpus")), 1);
});

forgetest_init!(forge_fuzz_cmin_counts_zero_replay_entries_once_per_corpus_entry, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminZeroReplayMultiTarget.t.sol",
        r#"
contract ForgeFuzzCminZeroReplayAlphaTest {
    function testFuzz_shared(uint256 input) public pure {
        if (input == 1) {
            revert("alpha rejects");
        }
    }
}

contract ForgeFuzzCminZeroReplayBetaTest {
    function testFuzz_shared(uint256 input) public pure {
        if (input == 1) {
            revert("beta rejects");
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzCminZeroReplayMultiTarget.t.sol/ForgeFuzzCminZeroReplayAlphaTest.json",
    );
    let one = calldata_for(&abi, "testFuzz_shared", 1);
    let corpus = prj.root().join("zero-replay-multi-target-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &one);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminZeroReplay(Alpha|Beta)Test",
            "--mt",
            "testFuzz_shared",
            "zero-replay-multi-target-corpus",
            "--corpus-out",
            "min-zero-replay-multi-target-corpus",
        ])
        .assert_failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("1 corpus entries failed during replay"), "{stderr}");
    assert!(!stderr.contains("2 corpus entries failed during replay"), "{stderr}");

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminZeroReplay(Alpha|Beta)Test",
            "--mt",
            "testFuzz_shared",
            "--sender",
            "0x000000000000000000000000000000000000dEaD",
            "zero-replay-multi-target-corpus",
            "--corpus-out",
            "min-zero-replay-stale-multi-target-corpus",
        ])
        .assert_failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("1 transactions did not match the test"), "{stderr}");
    assert!(!stderr.contains("2 transactions did not match the test"), "{stderr}");
});

forgetest_init!(forge_fuzz_cmin_rejects_stale_stateless_target, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminStaleTarget.t.sol",
        r#"
contract ForgeFuzzCminStaleTargetTest {
    uint256 value;

    function testFuzz_branch(uint256 input) public {
        if (input == 1) {
            value = 1;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzCminStaleTarget.t.sol/ForgeFuzzCminStaleTargetTest.json",
    );
    let one = calldata_for(&abi, "testFuzz_branch", 1);
    let corpus = prj.root().join("stale-target-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &one);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminStaleTargetTest",
            "--mt",
            "testFuzz_branch",
            "stale-target-corpus",
            "--corpus-out",
            "min-stale-target-corpus",
        ])
        .assert_success();

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminStaleTargetTest",
            "--mt",
            "testFuzz_branch",
            "--sender",
            "0x000000000000000000000000000000000000dEaD",
            "stale-target-corpus",
            "--corpus-out",
            "min-wrong-sender-corpus",
        ])
        .assert_failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("replayed 0 transactions from stale-target-corpus"), "{stderr}");
    assert!(stderr.contains("replay-critical options"), "{stderr}");
    assert!(!prj.root().join("min-wrong-sender-corpus").exists());
});

forgetest_init!(forge_fuzz_cmin_reports_zero_replay_reasons, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminZeroReplay.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzCminZeroReplayTest is Test {
    function testFuzz_assumeAlways(uint256 input) public {
        input;
        vm.assume(false);
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzCminZeroReplay.t.sol/ForgeFuzzCminZeroReplayTest.json",
    );
    let assume = calldata_for(&abi, "testFuzz_assumeAlways", 1);
    let assume_corpus = prj.root().join("assume-corpus");
    std::fs::create_dir_all(&assume_corpus).unwrap();
    write_corpus_entry(&assume_corpus, "00000000-0000-0000-0000-000000000001-1.json", &assume);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminZeroReplayTest",
            "--mt",
            "testFuzz_assumeAlways",
            "assume-corpus",
            "--corpus-out",
            "min-assume-corpus",
        ])
        .assert_failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("1 transactions were rejected by vm.assume or vm.skip"), "{stderr}");
    assert!(!stderr.contains("replay-critical options"), "{stderr}");
    assert!(!prj.root().join("min-assume-corpus").exists());

    let empty_corpus = prj.root().join("empty-corpus");
    std::fs::create_dir_all(&empty_corpus).unwrap();
    std::fs::write(empty_corpus.join("00000000-0000-0000-0000-000000000002-2.json"), "[]").unwrap();

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminZeroReplayTest",
            "--mt",
            "testFuzz_assumeAlways",
            "empty-corpus",
            "--corpus-out",
            "min-empty-corpus",
        ])
        .assert_failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("corpus entries were empty"), "{stderr}");
    assert!(!stderr.contains("replay-critical options"), "{stderr}");
    assert!(!prj.root().join("min-empty-corpus").exists());
});

forgetest_init!(forge_fuzz_cmin_minimizes_invariant_corpus, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminInvariantTarget.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzCminInvariantTargetTest is Test {
    uint256 value;

    function setUp() public {
        targetContract(address(this));
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = this.touch.selector;
        targetSelector(FuzzSelector({addr: address(this), selectors: selectors}));
    }

    function touch(uint256 input) external {
        if (input == 1) {
            value = 1;
        }
        if (input == 2) {
            value = 2;
        }
    }

    function invariant_ok() public view {
        assertTrue(value <= 2);
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzCminInvariantTarget.t.sol/ForgeFuzzCminInvariantTargetTest.json",
    );
    let one = calldata_for(&abi, "touch", 1);
    let two = calldata_for(&abi, "touch", 2);
    let corpus = prj.root().join("invariant-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &one);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000002-2.json", &one);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000003-3.json", &two);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzCminInvariantTargetTest",
            "--mt",
            "invariant_ok",
            "invariant-corpus",
            "--corpus-out",
            "min-invariant-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("minimized corpus: kept 2/3 entries in min-invariant-corpus"),
        "{stdout}"
    );
    assert_eq!(regular_file_count(&prj.root().join("min-invariant-corpus")), 2);
});

forgetest_init!(forge_fuzz_tmin_removes_redundant_transactions, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminRemoveTarget.t.sol",
        r#"
contract ForgeFuzzTminRemoveTargetTest {
    uint256 value;

    function testFuzz_samePath(uint256 input) public {
        if (input < 100) {
            value = 1;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminRemoveTarget.t.sol/ForgeFuzzTminRemoveTargetTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_samePath", 42);
    let corpus = prj.root().join("tmin-remove-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_sequence_entry(
        &corpus,
        "00000000-0000-0000-0000-000000000001-1.json",
        &[&calldata, &calldata],
    );

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminRemoveTargetTest",
            "--mt",
            "testFuzz_samePath",
            "tmin-remove-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-remove-output.json",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stdout.contains("minimized entry: 2 txs -> 1 txs in tmin-remove-output.json"),
        "{stdout}"
    );
    assert!(!stdout.contains("attempted"), "{stdout}");
    assert!(stderr.contains("attempted"), "{stderr}");

    let output: Value = serde_json::from_str(
        &std::fs::read_to_string(prj.root().join("tmin-remove-output.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(output.as_array().unwrap().len(), 1);
});

forgetest_init!(forge_fuzz_tmin_simplifies_abi_calldata, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminAbiTarget.t.sol",
        r#"
contract ForgeFuzzTminAbiTargetTest {
    uint256 value;

    function testFuzz_small(uint256 input) public {
        if (input < 100) {
            value = 1;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminAbiTarget.t.sol/ForgeFuzzTminAbiTargetTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_small", 42);
    let corpus = prj.root().join("tmin-abi-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminAbiTargetTest",
            "--mt",
            "testFuzz_small",
            "tmin-abi-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-abi-output.json",
        ])
        .assert_success();

    let output: Value = serde_json::from_str(
        &std::fs::read_to_string(prj.root().join("tmin-abi-output.json")).unwrap(),
    )
    .unwrap();
    let minimized = output[0]["calldata"].as_str().unwrap();
    assert!(
        minimized.ends_with("0000000000000000000000000000000000000000000000000000000000000000")
            || minimized
                .ends_with("0000000000000000000000000000000000000000000000000000000000000001"),
        "{minimized}"
    );
    assert!(
        !minimized.ends_with("000000000000000000000000000000000000000000000000000000000000002a")
    );
});

forgetest_init!(forge_fuzz_tmin_rejects_extra_coverage_edges, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminExactEdgesTarget.t.sol",
        r#"
contract ForgeFuzzTminExactEdgesTargetTest {
    uint256 value;

    function testFuzz_exactEdges(uint256 input) public {
        if (input < 100) {
            value = 1;
        }
        if (input == 0) {
            value = 2;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminExactEdgesTarget.t.sol/ForgeFuzzTminExactEdgesTargetTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_exactEdges", 42);
    let corpus = prj.root().join("tmin-exact-edges-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminExactEdgesTargetTest",
            "--mt",
            "testFuzz_exactEdges",
            "tmin-exact-edges-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-exact-edges-output.json",
        ])
        .assert_success();

    let args = output_calldata_args(
        prj.root(),
        "tmin-exact-edges-output.json",
        &abi,
        "testFuzz_exactEdges",
    );
    assert_eq!(args, vec![DynSolValue::Uint(U256::from(1), 256)]);
});

forgetest_init!(forge_fuzz_tmin_keeps_multiple_args_simplified, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminMultiArgTarget.t.sol",
        r#"
contract ForgeFuzzTminMultiArgTargetTest {
    uint256 value;

    function testFuzz_multi(uint256 left, uint256 right) public {
        if (left < 100) {
            value = 1;
        }
        if (right < 100) {
            value = 2;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminMultiArgTarget.t.sol/ForgeFuzzTminMultiArgTargetTest.json",
    );
    let calldata = calldata_for_args(
        &abi,
        "testFuzz_multi",
        &[DynSolValue::Uint(U256::from(42), 256), DynSolValue::Uint(U256::from(43), 256)],
    );
    let corpus = prj.root().join("tmin-multi-arg-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminMultiArgTargetTest",
            "--mt",
            "testFuzz_multi",
            "tmin-multi-arg-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-multi-arg-output.json",
        ])
        .assert_success();

    let args =
        output_calldata_args(prj.root(), "tmin-multi-arg-output.json", &abi, "testFuzz_multi");
    assert_eq!(args, vec![DynSolValue::Uint(U256::ZERO, 256), DynSolValue::Uint(U256::ZERO, 256)]);
});

forgetest_init!(forge_fuzz_tmin_keeps_array_length_reduction, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminArrayTarget.t.sol",
        r#"
contract ForgeFuzzTminArrayTargetTest {
    uint256 value;

    function testFuzz_array(uint256[] memory inputs) public {
        if (inputs.length <= 4) {
            value = 1;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminArrayTarget.t.sol/ForgeFuzzTminArrayTargetTest.json",
    );
    let calldata = calldata_for_args(
        &abi,
        "testFuzz_array",
        &[DynSolValue::Array(vec![
            DynSolValue::Uint(U256::from(10), 256),
            DynSolValue::Uint(U256::from(11), 256),
            DynSolValue::Uint(U256::from(12), 256),
            DynSolValue::Uint(U256::from(13), 256),
        ])],
    );
    let corpus = prj.root().join("tmin-array-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminArrayTargetTest",
            "--mt",
            "testFuzz_array",
            "tmin-array-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-array-output.json",
        ])
        .assert_success();

    let args = output_calldata_args(prj.root(), "tmin-array-output.json", &abi, "testFuzz_array");
    let [DynSolValue::Array(values)] = args.as_slice() else {
        panic!("expected one array argument, got {args:?}");
    };
    assert!(values.len() < 4, "{values:?}");
});

forgetest_init!(forge_fuzz_tmin_preserves_fuzz_failure_identity, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminFailureTarget.t.sol",
        r#"
contract ForgeFuzzTminFailureTargetTest {
    function testFuzz_failure(uint256 input) public pure {
        if (input == 0) {
            revert("zero");
        }
        if (input == 1) {
            revert("one");
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminFailureTarget.t.sol/ForgeFuzzTminFailureTargetTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_failure", 1);
    let corpus = prj.root().join("tmin-failure-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminFailureTargetTest",
            "--mt",
            "testFuzz_failure",
            "tmin-failure-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-failure-output.json",
        ])
        .assert_success();

    let output: Value = serde_json::from_str(
        &std::fs::read_to_string(prj.root().join("tmin-failure-output.json")).unwrap(),
    )
    .unwrap();
    let minimized = output[0]["calldata"].as_str().unwrap();
    assert!(
        minimized.ends_with("0000000000000000000000000000000000000000000000000000000000000001"),
        "{minimized}"
    );
});

forgetest_init!(forge_fuzz_tmin_rejects_existing_output, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminExistingOutput.t.sol",
        r#"
contract ForgeFuzzTminExistingOutputTest {
    function testFuzz_value(uint256 input) public pure {
        input;
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminExistingOutput.t.sol/ForgeFuzzTminExistingOutputTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_value", 1);
    let corpus = prj.root().join("tmin-existing-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);
    std::fs::write(prj.root().join("tmin-existing-output.json"), "keep").unwrap();

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminExistingOutputTest",
            "--mt",
            "testFuzz_value",
            "tmin-existing-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-existing-output.json",
        ])
        .assert_failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("output corpus file already exists"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(prj.root().join("tmin-existing-output.json")).unwrap(),
        "keep"
    );
});

forgetest_init!(forge_fuzz_tmin_minimizes_corpus_directory, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminDirectoryTarget.t.sol",
        r#"
contract ForgeFuzzTminDirectoryTargetTest {
    uint256 value;

    function testFuzz_directory(uint256 input) public {
        if (input < 100) {
            value = 1;
        } else {
            value = 2;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminDirectoryTarget.t.sol/ForgeFuzzTminDirectoryTargetTest.json",
    );
    let small = calldata_for(&abi, "testFuzz_directory", 42);
    let large = calldata_for(&abi, "testFuzz_directory", 142);
    let corpus = prj.root().join("tmin-dir-corpus/Target/testFuzz_directory/worker0/corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &small);
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000002-2.json", &large);
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000003-3.json"), "[]").unwrap();
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000004-4.json"), "not json").unwrap();

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminDirectoryTargetTest",
            "--mt",
            "testFuzz_directory",
            "tmin-dir-corpus",
            "--corpus-out",
            "tmin-dir-output",
        ])
        .assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stdout.contains("minimized corpus: 2 entries"), "{stdout}");
    assert!(!stdout.contains("attempted"), "{stdout}");
    assert!(stderr.contains("attempted"), "{stderr}");
    assert!(stderr.contains("skipped 2 entries"), "{stderr}");
    assert_eq!(regular_file_count(&prj.root().join("tmin-dir-output")), 2);

    let show = cmd.forge_fuse().args(["fuzz", "show", "tmin-dir-output"]).assert_success();
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("ForgeFuzzTminDirectoryTargetTest.testFuzz_directory"), "{stdout}");
});

forgetest_init!(forge_fuzz_tmin_writes_gzip_output, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminGzipTarget.t.sol",
        r#"
contract ForgeFuzzTminGzipTargetTest {
    uint256 value;

    function testFuzz_gzip(uint256 input) public {
        if (input < 100) {
            value = 1;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminGzipTarget.t.sol/ForgeFuzzTminGzipTargetTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_gzip", 42);
    let corpus = prj.root().join("tmin-gzip-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminGzipTargetTest",
            "--mt",
            "testFuzz_gzip",
            "tmin-gzip-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-gzip-output.json.gz",
        ])
        .assert_success();

    let show = cmd.forge_fuse().args(["fuzz", "show", "tmin-gzip-output.json.gz"]).assert_success();
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("ForgeFuzzTminGzipTargetTest.testFuzz_gzip"), "{stdout}");
});

forgetest_init!(forge_fuzz_tmin_rejects_zero_attempt_budget, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminBudgetTarget.t.sol",
        r#"
contract ForgeFuzzTminBudgetTargetTest {
    function testFuzz_budget(uint256 input) public pure {
        input;
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminBudgetTarget.t.sol/ForgeFuzzTminBudgetTargetTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_budget", 42);
    let corpus = prj.root().join("tmin-budget-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminBudgetTargetTest",
            "--mt",
            "testFuzz_budget",
            "tmin-budget-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-budget-output.json",
            "--max-attempts",
            "0",
        ])
        .assert_failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("--max-attempts must be greater than 0"), "{stderr}");
    assert!(!prj.root().join("tmin-budget-output.json").exists());
});

forgetest_init!(forge_fuzz_tmin_rejects_unreplayable_entry, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminUnreplayableTarget.t.sol",
        r#"
contract ForgeFuzzTminUnreplayableTargetTest {
    function testFuzz_value(uint256 input) public pure {
        input;
    }

    function testFuzz_other(uint256 input) public pure {
        input;
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzTminUnreplayableTarget.t.sol/ForgeFuzzTminUnreplayableTargetTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_other", 1);
    let corpus = prj.root().join("tmin-unreplayable-corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    let assert = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminUnreplayableTargetTest",
            "--mt",
            "testFuzz_value",
            "tmin-unreplayable-corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--corpus-out",
            "tmin-unreplayable-output.json",
        ])
        .assert_failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("replayed 0 transactions"), "{stderr}");
    assert!(!prj.root().join("tmin-unreplayable-output.json").exists());
});

forgetest_init!(forge_fuzz_replay_invariant_fail_on_revert, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
    });
    prj.add_test(
        "ForgeFuzzInvariantFailOnRevertReplay.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzInvariantFailOnRevertReplayTest is Test {
    bool ok = true;

    function setUp() public {
        targetContract(address(this));
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = this.revertHandler.selector;
        selectors[1] = this.breakInvariant.selector;
        targetSelector(FuzzSelector({addr: address(this), selectors: selectors}));
    }

    function revertHandler(uint256 value) external pure {
        value;
        revert("boom");
    }

    function breakInvariant() external {
        ok = false;
    }

    function invariant_ok() public view {
        assertTrue(ok);
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzInvariantFailOnRevertReplay.t.sol/ForgeFuzzInvariantFailOnRevertReplayTest.json",
    );
    let revert_handler = calldata_for(&abi, "revertHandler", 1);
    let break_invariant = format!(
        "0x{}",
        hex::encode(
            abi.functions().find(|function| function.name == "breakInvariant").unwrap().selector()
        )
    );
    let corpus = prj.root().join("invariant_corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    std::fs::write(
        corpus.join("00000000-0000-0000-0000-000000000001-1.json"),
        format!(
            r#"[{{
  "sender":"{DEFAULT_SENDER}",
  "target":"{DEFAULT_TEST_TARGET}",
  "calldata":"{revert_handler}",
  "value":"0x0"
}},{{
  "sender":"{DEFAULT_SENDER}",
  "target":"{DEFAULT_TEST_TARGET}",
  "calldata":"{break_invariant}",
  "value":"0x0"
}}]"#
        ),
    )
    .unwrap();

    let replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzInvariantFailOnRevertReplayTest",
            "--mt",
            "invariant_ok",
            "--corpus-dir",
            "invariant_corpus",
        ])
        .assert_failure();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("failed during replay: invariant `invariant_ok` failed on handler "),
        "{stdout}"
    );

    let showmap = cmd
        .forge_fuse()
        .args([
            "test",
            "--mc",
            "ForgeFuzzInvariantFailOnRevertReplayTest",
            "--mt",
            "invariant_ok",
            "--showmap-out",
            "showmap",
            "--showmap-corpus-dir",
            "invariant_corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(showmap.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[PASS] invariant_ok() (replay: 1 entries"), "{stdout}");
});

forgetest_init!(forge_fuzz_replay_invariant_sequence_checks, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(1u32));
        config.invariant.runs = 1;
        config.invariant.depth = 1;
        config.invariant.check_interval = 0;
        config.invariant.corpus.corpus_dir = Some("seed_corpus".into());
        config.invariant.corpus.corpus_gzip = false;
    });
    prj.add_test(
        "ForgeFuzzInvariantReplaySequence.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzInvariantReplaySequenceTest is Test {
    bool checksEnabled;
    bool ok = true;
    bool afterFails;

    function setUp() public {
        targetContract(address(this));
        bytes4[] memory selectors = new bytes4[](3);
        selectors[0] = this.enableChecks.selector;
        selectors[1] = this.setOk.selector;
        selectors[2] = this.armAfterInvariant.selector;
        targetSelector(FuzzSelector({addr: address(this), selectors: selectors}));
    }

    function enableChecks() external {
        checksEnabled = true;
    }

    function setOk(bool value) external {
        ok = value;
    }

    function armAfterInvariant(uint256 value) external {
        if (value == 0xdeadbeef) {
            afterFails = true;
        }
    }

    function invariant_ok() public view {
        if (checksEnabled) {
            assertTrue(ok);
        }
    }

    function afterInvariant() external view {
        if (checksEnabled && !ok) {
            revert("after");
        }
        if (afterFails) {
            revert("after");
        }
    }
}
   "#,
    );
    cmd.args([
        "test",
        "--mc",
        "ForgeFuzzInvariantReplaySequenceTest",
        "--mt",
        "invariant_ok",
        "-q",
    ])
    .assert_success();

    let seed_entry =
        std::fs::read_to_string(find_first_json(&prj.root().join("seed_corpus"))).unwrap();
    let seed_entry: Value = serde_json::from_str(&seed_entry).unwrap();
    let tx = &seed_entry.as_array().unwrap()[0];
    let sender = tx["sender"].as_str().unwrap();
    let target = tx["target"].as_str().unwrap();
    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzInvariantReplaySequence.t.sol/ForgeFuzzInvariantReplaySequenceTest.json",
    );
    let selector = |name: &str| {
        let function = abi.functions().find(|function| function.name == name).unwrap();
        hex::encode(function.selector())
    };
    let enable_checks = format!("0x{}", selector("enableChecks"));
    let set_ok_false = format!("0x{}{:064x}", selector("setOk"), 0);
    let set_ok_true = format!("0x{}{:064x}", selector("setOk"), 1);
    let arm_after_invariant = format!("0x{}{:064x}", selector("armAfterInvariant"), 0xdeadbeefu64);
    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let entry = format!(
        r#"[{{
  "sender":"{sender}",
  "target":"{target}",
  "calldata":"{enable_checks}",
  "value":"0x0"
}},{{
  "sender":"{sender}",
  "target":"{target}",
  "calldata":"{set_ok_false}",
  "value":"0x0"
}},{{
  "sender":"{sender}",
  "target":"{target}",
  "calldata":"{set_ok_true}",
  "value":"0x0"
}}]"#
    );
    std::fs::write(corpus.join("check-interval.json"), entry).unwrap();
    let entry = format!(
        r#"[{{
  "sender":"{sender}",
  "target":"{target}",
  "calldata":"{arm_after_invariant}",
  "value":"0x0"
}}]"#
    );
    std::fs::write(corpus.join("after-invariant.json"), entry).unwrap();

    let replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzInvariantReplaySequenceTest",
            "--mt",
            "invariant_ok",
            "--corpus-dir",
            "corpus/check-interval.json",
        ])
        .assert_success();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[PASS] invariant_ok() (replay: 1 entries"), "{stdout}");

    let replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzInvariantReplaySequenceTest",
            "--mt",
            "invariant_ok",
            "--corpus-dir",
            "corpus/after-invariant.json",
        ])
        .assert_failure();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("failed during replay: afterInvariant broken"), "{stdout}");
});

forgetest_init!(forge_fuzz_corpus_subcommands_dedup_worker_entries, |prj, cmd| {
    let worker0 = prj.root().join("corpus/worker0/corpus");
    let worker1 = prj.root().join("corpus/worker1/corpus");
    std::fs::create_dir_all(&worker0).unwrap();
    std::fs::create_dir_all(&worker1).unwrap();
    let entry = r#"[{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x0000000000000000000000000000000000000002",
  "calldata":"0x12345678"
}]"#;
    let name = "00000000-0000-0000-0000-000000000001-1.json";
    std::fs::write(worker0.join(name), entry).unwrap();
    std::fs::write(worker1.join(name), entry).unwrap();

    let show = cmd.args(["fuzz", "show", "corpus"]).assert_success();
    let stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    assert_eq!(
        stdout.matches(&format!("corpus{}worker", std::path::MAIN_SEPARATOR)).count(),
        1,
        "{stdout}"
    );
});

forgetest_init!(forge_fuzz_replay_error_on_zero_replay, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzZeroReplay.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzZeroReplayTest is Test {
    function testFuzz_branch(uint256 value) public pure {
        value;
    }

    function testFuzz_assumeAlways(uint256 value) public {
        value;
        vm.assume(false);
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(
        &corpus,
        "00000000-0000-0000-0000-000000000001-1.json",
        "0x003919a00000000000000000000000000000000000000000000000000000000000000001",
    );

    let wrong_corpus = prj.root().join("wrong-corpus");
    std::fs::create_dir_all(&wrong_corpus).unwrap();
    write_corpus_entry(
        &wrong_corpus,
        "00000000-0000-0000-0000-000000000002-2.json",
        "0xdeadbeef0000000000000000000000000000000000000000000000000000000000000001",
    );

    let zero_replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzZeroReplayTest",
            "--mt",
            "testFuzz_branch",
            "--corpus-dir",
            "wrong-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(zero_replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[SKIP: replayed 0 corpus entries from wrong-corpus]"), "{stdout}");
    assert!(!stdout.contains("[PASS] testFuzz_branch(uint256) (replay: 0 entries"), "{stdout}");

    let abi =
        artifact_abi(prj.root(), "out/ForgeFuzzZeroReplay.t.sol/ForgeFuzzZeroReplayTest.json");
    let assume_corpus = prj.root().join("assume-corpus");
    std::fs::create_dir_all(&assume_corpus).unwrap();
    write_corpus_entry(
        &assume_corpus,
        "00000000-0000-0000-0000-000000000003-3.json",
        &calldata_for(&abi, "testFuzz_assumeAlways", 1),
    );
    let all_assume_replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzZeroReplayTest",
            "--mt",
            "testFuzz_assumeAlways",
            "--corpus-dir",
            "assume-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(all_assume_replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[SKIP: replayed 0 corpus entries from assume-corpus]"), "{stdout}");
    assert!(
        !stdout.contains("[PASS] testFuzz_assumeAlways(uint256) (replay: 1 entries"),
        "{stdout}"
    );
    let all_assume_showmap = cmd
        .forge_fuse()
        .args([
            "test",
            "--mc",
            "ForgeFuzzZeroReplayTest",
            "--mt",
            "testFuzz_assumeAlways",
            "--showmap-out",
            "assume-showmap",
            "--showmap-corpus-dir",
            "assume-corpus",
            "--showmap-per-input",
        ])
        .assert_success();
    let stdout = String::from_utf8(all_assume_showmap.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("(replay: 0 entries, 0 files, 1 skipped)"), "{stdout}");
    assert!(!has_regular_file(&prj.root().join("assume-showmap")));

    prj.add_test(
        "ForgeFuzzSkipReplay.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzSkipReplayTest is Test {
    function testFuzz_skipEven(uint256 value) public {
        if (value % 2 == 0) {
            vm.skip(true, "even");
        }
    }
}
   "#,
    );
    cmd.forge_fuse().args(["build", "-q"]).assert_success();

    let abi =
        artifact_abi(prj.root(), "out/ForgeFuzzSkipReplay.t.sol/ForgeFuzzSkipReplayTest.json");
    let skip_corpus = prj.root().join("skip-corpus");
    std::fs::create_dir_all(&skip_corpus).unwrap();
    write_corpus_entry(
        &skip_corpus,
        "00000000-0000-0000-0000-000000000004-4.json",
        &calldata_for(&abi, "testFuzz_skipEven", 8),
    );
    let all_skip_replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzSkipReplayTest",
            "--mt",
            "testFuzz_skipEven",
            "--corpus-dir",
            "skip-corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(all_skip_replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[SKIP: replayed 0 corpus entries from skip-corpus]"), "{stdout}");
    assert!(!stdout.contains("corpus replay failed"), "{stdout}");

    let malformed_corpus = prj.root().join("malformed-corpus");
    std::fs::create_dir_all(&malformed_corpus).unwrap();
    std::fs::write(malformed_corpus.join("00000000-0000-0000-0000-000000000004-4.json"), "{")
        .unwrap();
    write_corpus_entry(
        &malformed_corpus,
        "00000000-0000-0000-0000-000000000005-5.json",
        "0xdeadbeef0000000000000000000000000000000000000000000000000000000000000001",
    );
    let malformed_replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzZeroReplayTest",
            "--mt",
            "testFuzz_branch",
            "--corpus-dir",
            "malformed-corpus",
        ])
        .assert_failure();
    let stdout = String::from_utf8(malformed_replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("failed to read 1 corpus entries from malformed-corpus"), "{stdout}");
});

forgetest_init!(forge_fuzz_commands_read_generated_corpus_roots, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.fuzz.runs = 8;
        config.fuzz.corpus.corpus_dir = Some("fuzz_corpus".into());
        config.fuzz.corpus.corpus_gzip = false;
        config.invariant.runs = 4;
        config.invariant.depth = 4;
        config.invariant.corpus.corpus_dir = Some("invariant_corpus".into());
        config.invariant.corpus.corpus_gzip = false;
    });
    prj.add_test(
        "ForgeFuzzGeneratedCorpus.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract ForgeFuzzGeneratedCorpusTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function test_unit() public pure {}

    function testFuzz_SetNumber(uint256 value) public {
        counter.setNumber(value);
        assertEq(counter.number(), value);
    }

    function invariant_counter_is_reachable() public view {}
}
   "#,
    );

    cmd.args(["fuzz", "run", "--mc", "ForgeFuzzGeneratedCorpusTest"]).assert_success();

    let show = cmd.forge_fuse().args(["fuzz", "show", "fuzz_corpus"]).assert_success();
    let show_stdout = String::from_utf8(show.get_output().stdout.clone()).unwrap();
    let fuzz_corpus_path = ["fuzz_corpus", "ForgeFuzzGeneratedCorpusTest", "testFuzz_SetNumber"]
        .join(std::path::MAIN_SEPARATOR_STR);
    assert!(show_stdout.contains(&fuzz_corpus_path), "{show_stdout}");

    let replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzGeneratedCorpusTest",
            "--corpus-dir",
            "fuzz_corpus",
        ])
        .assert_success();
    let replay_stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(
        replay_stdout.contains("[PASS] testFuzz_SetNumber(uint256) (replay:"),
        "{replay_stdout}"
    );
    assert!(
        !replay_stdout.contains("[PASS] testFuzz_SetNumber(uint256) (replay: 0 entries"),
        "{replay_stdout}"
    );

    let invariant_show =
        cmd.forge_fuse().args(["fuzz", "show", "invariant_corpus"]).assert_success();
    let invariant_stdout = String::from_utf8(invariant_show.get_output().stdout.clone()).unwrap();
    let invariant_corpus_path =
        ["invariant_corpus", "ForgeFuzzGeneratedCorpusTest", "worker0", "corpus"]
            .join(std::path::MAIN_SEPARATOR_STR);
    assert!(invariant_stdout.contains(&invariant_corpus_path), "{invariant_stdout}");
});

forgetest_init!(fuzz_branch_frontiers_capture_comparison_for_symbolic_followup, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzFrontier.t.sol",
        r#"
contract ForgeFuzzFrontierTest {
    function testFuzz_frontier(uint64 amount, uint256 feeMultiplier) public pure {
        uint256 credited;
        unchecked {
            credited = uint256(amount) + (feeMultiplier - 100);
        }

        if (feeMultiplier < 100) {
            assert(credited <= amount);
        }
    }
}
   "#,
    );

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_frontier",
            "--fuzz-runs",
            "8",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
        ])
        .assert_success();

    let assert_frontier_artifact = |frontier_dir: &str, expect_new_coverage: bool| {
        let frontier_path = prj
            .root()
            .join(frontier_dir)
            .join("ForgeFuzzFrontierTest")
            .join("testFuzz_frontier")
            .join("branch-frontiers.json");
        let artifact: Value = serde_json::from_slice(
            &std::fs::read(&frontier_path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", frontier_path.display())),
        )
        .unwrap();
        assert_eq!(artifact["schema"], "foundry:fuzz.branch-frontiers@v1");
        assert_eq!(artifact["version"], 1);
        assert_eq!(artifact["test"], "testFuzz_frontier(uint64,uint256)");
        assert_eq!(artifact["limit"], 256);

        let frontiers = artifact["frontiers"].as_array().unwrap();
        let frontier = frontiers
            .iter()
            .find(|frontier| {
                frontier["site"]["opcode_name"] == "LT"
                    && frontier["operands"]["lhs"] == "0x64"
                    && frontier["operands"]["rhs"] == "0x64"
                    && frontier["operands"]["result"] == false
            })
            .unwrap_or_else(|| panic!("missing missed fee multiplier frontier in {artifact:#}"));
        assert!(frontier["id"].as_u64().is_some(), "{frontier:#}");
        assert_eq!(frontier["call_index"], 0);
        assert_eq!(frontier["site"]["opcode_name"], "LT");
        assert!(frontier["site"]["pc"].as_u64().is_some(), "{frontier:#}");
        assert_eq!(frontier["operands"]["operand_delta"], "0x0");
        assert_eq!(frontier["operands"]["result"], false);

        // `new_coverage` is only recorded when edge coverage is collected; frontier-only capture
        // omits it rather than reporting an always-false value.
        if expect_new_coverage {
            assert!(frontier["new_coverage"].is_boolean(), "{frontier:#}");
        } else {
            assert!(frontier.get("new_coverage").is_none(), "{frontier:#}");
        }

        let sequence = frontier["sequence"].as_array().unwrap();
        assert_eq!(sequence.len(), 1);
        assert!(
            sequence[0]["target"].as_str().unwrap().eq_ignore_ascii_case(DEFAULT_TEST_TARGET),
            "{frontier:#}"
        );
        let calldata = sequence[0]["calldata"].as_str().unwrap();
        let expected_selector =
            hex::encode(&alloy_primitives::keccak256(b"testFuzz_frontier(uint64,uint256)")[..4]);
        assert!(calldata.starts_with(&format!("0x{expected_selector}")), "{calldata}");
        assert!(calldata.ends_with(&format!("{:064x}{:064x}", 100u64, 100u64)), "{calldata}");
    };
    assert_frontier_artifact("fuzz_frontiers", false);

    prj.update_config(|config| {
        config.fuzz.corpus.sancov_edges = true;
    });
    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_frontier",
            "--fuzz-runs",
            "8",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "sancov_fuzz_frontiers",
        ])
        .assert_success();
    assert_frontier_artifact("sancov_fuzz_frontiers", true);
});

forgetest_init!(forge_fuzz_replay_scopes_generated_corpus_root_to_target, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzGeneratedRootScope.t.sol",
        r#"
contract GeneratedCorpusATest {
    function testFuzz_same(uint256 value) public pure {
        require(value == 1);
    }
}

contract GeneratedCorpusBTest {
    function testFuzz_same(uint256 value) public pure {
        require(value == 2);
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi =
        artifact_abi(prj.root(), "out/ForgeFuzzGeneratedRootScope.t.sol/GeneratedCorpusBTest.json");
    let selector = calldata_for(&abi, "testFuzz_same", 0);
    let selector = &selector[..10];

    let corpus_root = prj.root().join("fuzz_corpus");
    let a_corpus = corpus_root.join("GeneratedCorpusATest/testFuzz_same/worker0/corpus");
    let b_corpus = corpus_root.join("GeneratedCorpusBTest/testFuzz_same/worker0/corpus");
    std::fs::create_dir_all(&a_corpus).unwrap();
    std::fs::create_dir_all(&b_corpus).unwrap();
    write_corpus_entry(
        &a_corpus,
        "00000000-0000-0000-0000-000000000001-1.json",
        &format!("{selector}{:064x}", 1),
    );
    write_corpus_entry(
        &b_corpus,
        "00000000-0000-0000-0000-000000000002-2.json",
        &format!("{selector}{:064x}", 2),
    );

    let replay = cmd
        .forge_fuse()
        .args(["fuzz", "replay", "--mc", "GeneratedCorpusBTest", "--corpus-dir", "fuzz_corpus"])
        .assert_success();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[PASS] testFuzz_same(uint256) (replay: 1 entries"), "{stdout}");
    assert!(!stdout.contains("corpus replay failed"), "{stdout}");
});

forgetest_init!(forge_fuzz_replay_scopes_generated_invariant_root_to_target, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
    });
    prj.add_test(
        "ForgeFuzzGeneratedInvariantRootScope.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract GeneratedInvariantCorpusATest is Test {
    function setUp() public {
        targetContract(address(this));
    }

    function setValue(uint256 value) external {
        require(value == 1);
    }

    function invariant_ok() public pure {}
}

contract GeneratedInvariantCorpusBTest is Test {
    function setUp() public {
        targetContract(address(this));
    }

    function setValue(uint256 value) external {
        require(value == 2);
    }

    function invariant_ok() public pure {}
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzGeneratedInvariantRootScope.t.sol/GeneratedInvariantCorpusBTest.json",
    );
    let corpus_root = prj.root().join("invariant_corpus");
    let a_corpus = corpus_root.join("GeneratedInvariantCorpusATest/worker0/corpus");
    let b_corpus = corpus_root.join("GeneratedInvariantCorpusBTest/worker0/corpus");
    std::fs::create_dir_all(&a_corpus).unwrap();
    std::fs::create_dir_all(&b_corpus).unwrap();
    write_corpus_entry(
        &a_corpus,
        "00000000-0000-0000-0000-000000000001-1.json",
        &calldata_for(&abi, "setValue", 1),
    );
    write_corpus_entry(
        &b_corpus,
        "00000000-0000-0000-0000-000000000002-2.json",
        &calldata_for(&abi, "setValue", 2),
    );

    let replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "GeneratedInvariantCorpusBTest",
            "--mt",
            "invariant_ok",
            "--corpus-dir",
            "invariant_corpus",
        ])
        .assert_success();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[PASS] invariant_ok() (replay: 1 entries"), "{stdout}");
    assert!(!stdout.contains("corpus replay failed"), "{stdout}");
});

// tests that inline max-test-rejects config is properly applied
forgetest_init!(test_inline_max_test_rejects, |prj, cmd| {
    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract InlineMaxRejectsTest is Test {
    /// forge-config: default.fuzz.max-test-rejects = 1
    function test_fuzz_bound(uint256 a) public {
        vm.assume(false);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: `vm.assume` rejected too many inputs (1 allowed)] test_fuzz_bound(uint256) (runs: 0, [AVG_GAS])
...
"#]]);
});

// Tests that test timeout config is properly applied.
// If test doesn't timeout after one second, then test will fail with `rejected too many inputs`.
forgetest_init!(test_fuzz_timeout, |prj, cmd| {
    prj.add_test(
        "Contract.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FuzzTimeoutTest is Test {
    /// forge-config: default.fuzz.max-test-rejects = 0
    /// forge-config: default.fuzz.timeout = 1
    function test_fuzz_bound(uint256 a) public pure {
        vm.assume(a == 0);
    }
}
   "#,
    );

    cmd.args(["test", "-j2"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Contract.t.sol:FuzzTimeoutTest
[PASS] test_fuzz_bound(uint256) (runs: [..], [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest_init!(test_fuzz_fail_on_revert, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.fail_on_revert = false;
        config.fuzz.seed = Some(U256::from(100u32));
    });
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        require(number > 10000000000, "low number");
        number = newNumber;
    }
}
   "#,
    );

    prj.add_test(
        "CounterTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import "src/Counter.sol";

contract CounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function testFuzz_SetNumberRequire(uint256 x) public {
        counter.setNumber(x);
        require(counter.number() == 1);
    }

    function testFuzz_SetNumberAssert(uint256 x) public {
        counter.setNumber(x);
        assertEq(counter.number(), 1);
    }
}
   "#,
    );

    // Tests should not fail as revert happens in Counter contract.
    cmd.args(["test", "--mc", "CounterTest"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/CounterTest.t.sol:CounterTest
[PASS] testFuzz_SetNumberAssert(uint256) (runs: 256, [AVG_GAS])
[PASS] testFuzz_SetNumberRequire(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);

    // Tested contract does not revert.
    prj.add_source(
        "Counter.sol",
        r#"
contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }
}
   "#,
    );

    // Tests should fail as revert happens in cheatcode (assert) and test (require) contract.
    cmd.args(["-j1"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/CounterTest.t.sol:CounterTest
[FAIL: assertion failed: [..]] testFuzz_SetNumberAssert(uint256) (runs: 0, [AVG_GAS])
[FAIL: EvmError: Revert; [..]] testFuzz_SetNumberRequire(uint256) (runs: 0, [AVG_GAS])
Suite result: FAILED. 0 passed; 2 failed; 0 skipped; [ELAPSED]
...

Ran 1 test suite [ELAPSED]: 0 tests passed, 2 failed, 0 skipped (2 total tests)

Failing tests:
Encountered 2 failing tests in test/CounterTest.t.sol:CounterTest
[FAIL: assertion failed: [..]; counterexample: calldata=[..] args=[..]] testFuzz_SetNumberAssert(uint256) (runs: 0, [AVG_GAS])
[FAIL: EvmError: Revert; counterexample: calldata=[..] args=[..]] testFuzz_SetNumberRequire(uint256) (runs: 0, [AVG_GAS])

Encountered a total of 2 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 2 failed tests
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
});

forgetest_init!(forge_fuzz_replay_respects_fuzz_fail_on_revert, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.fail_on_revert = true;
    });
    prj.add_source(
        "Reverter.sol",
        r#"
contract Reverter {
    function boom(uint256 value) public pure {
        value;
        revert("boom");
    }
}
   "#,
    );
    prj.add_test(
        "ForgeFuzzReplayFailOnRevert.t.sol",
        r#"
import {Reverter} from "../src/Reverter.sol";

contract ForgeFuzzReplayFailOnRevertTest {
    Reverter reverter;

    function setUp() public {
        reverter = new Reverter();
    }

    function testFuzz_callsReverter(uint256 value) public view {
        reverter.boom(value);
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let abi = artifact_abi(
        prj.root(),
        "out/ForgeFuzzReplayFailOnRevert.t.sol/ForgeFuzzReplayFailOnRevertTest.json",
    );
    let calldata = calldata_for(&abi, "testFuzz_callsReverter", 1);
    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    write_corpus_entry(&corpus, "00000000-0000-0000-0000-000000000001-1.json", &calldata);

    let replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzReplayFailOnRevertTest",
            "--corpus-dir",
            "corpus",
        ])
        .assert_failure();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("failed during replay: fuzz call"), "{stdout}");
});

forgetest_init!(forge_fuzz_replay_replays_persisted_handler_failures, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = false;
    });
    prj.add_source(
        "AlwaysAssert.sol",
        r#"
contract AlwaysAssert {
    function boom() external { assert(false); }
}
   "#,
    );
    prj.add_test(
        "ForgeFuzzReplayHandlerFailure.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {AlwaysAssert} from "../src/AlwaysAssert.sol";

contract ForgeFuzzReplayHandlerFailureTest is Test {
    AlwaysAssert h;

    function setUp() public {
        h = new AlwaysAssert();
        targetContract(address(h));
    }

    function invariant_ok() public view {}
}
   "#,
    );

    cmd.args(["test", "--mc", "ForgeFuzzReplayHandlerFailureTest", "--mt", "invariant_ok"])
        .assert_failure();

    let replay = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--mc",
            "ForgeFuzzReplayHandlerFailureTest",
            "--mt",
            "invariant_ok",
        ])
        .assert_failure();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Assertion Tests: 1 assertion bug(s) found"), "{stdout}");
    assert!(!stdout.contains("[SKIP: no persisted invariant failure reproduced"), "{stdout}");
});

forgetest_init!(forge_fuzz_replay_replays_non_anchor_invariant_failure, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 1;
    });
    prj.add_test(
        "ForgeFuzzReplayNonAnchorInvariant.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzReplayNonAnchorInvariantTest is Test {
    uint256 count;

    function setUp() public {
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = this.tick.selector;
        targetSelector(FuzzSelector({addr: address(this), selectors: selectors}));
    }

    function tick() external {
        count++;
    }

    function invariant_a_first() public view {
        require(count >= 0, "a");
    }

    function invariant_b_middle() public view {
        require(count < 1, "middle broken");
    }

    function invariant_c_last() public view {
        require(count >= 0, "c");
    }
}
   "#,
    );

    cmd.args(["test", "--mc", "ForgeFuzzReplayNonAnchorInvariantTest", "-q"]).assert_failure();
    let replay = cmd
        .forge_fuse()
        .args(["fuzz", "replay", "--mc", "ForgeFuzzReplayNonAnchorInvariantTest"])
        .assert_failure();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[FAIL: middle broken]"), "{stdout}");
    assert!(!stdout.contains("[SKIP: no persisted invariant failure reproduced"), "{stdout}");
});

// Test 256 runs regardless number of test rejects.
// <https://github.com/foundry-rs/foundry/issues/9054>
forgetest_init!(test_fuzz_runs_with_rejects, |prj, cmd| {
    prj.add_test(
        "FuzzWithRejectsTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract FuzzWithRejectsTest is Test {
    function testFuzzWithRejects(uint256 x) public pure {
        vm.assume(x < 1_000_000);
    }
}
   "#,
    );

    // Tests should not fail as revert happens in Counter contract.
    cmd.args(["test", "--mc", "FuzzWithRejectsTest"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/FuzzWithRejectsTest.t.sol:FuzzWithRejectsTest
[PASS] testFuzzWithRejects(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

// Test that counterexample is not replayed if test changes.
// <https://github.com/foundry-rs/foundry/issues/11927>
forgetest_init!(test_fuzz_replay_with_changed_test, |prj, cmd| {
    prj.update_config(|config| config.fuzz.seed = Some(U256::from(100u32)));
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint256 x) public pure {
        require(x > 200);
    }
}
   "#,
    );
    // Tests should fail and record counterexample with value 200.
    cmd.args(["test", "-j1"]).assert_failure().stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert; counterexample: calldata=0x5c7f60d700000000000000000000000000000000000000000000000000000000000000c8 args=[200]] testFuzz_SetNumber(uint256) (runs: 6, [AVG_GAS])
...

"#]]);

    // Change test to assume counterexample 2 is discarded.
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint256 x) public pure {
        vm.assume(x != 200);
    }
}
   "#,
    );
    // Test should pass when replay failure with changed assume logic.
    cmd.forge_fuse().args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Change test signature.
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint8 x) public pure {
    }
}
   "#,
    );
    // Test should pass when replay failure with changed function signature.
    cmd.forge_fuse().args(["test"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint8) (runs: 256, [AVG_GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);

    // Change test back to the original one that produced the counterexample.
    prj.add_test(
        "Counter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract CounterTest is Test {
    function testFuzz_SetNumber(uint256 x) public pure {
        require(x > 200);
    }
}
   "#,
    );
    // Test should fail with replayed counterexample 200 (0 runs).
    cmd.forge_fuse().args(["test", "-j1"]).assert_failure().stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Counter.t.sol:CounterTest
[FAIL: EvmError: Revert; counterexample: calldata=0x5c7f60d700000000000000000000000000000000000000000000000000000000000000c8 args=[200]] testFuzz_SetNumber(uint256) (runs: 0, [AVG_GAS])
...

"#]]);
});

forgetest_init!(fuzz_basic, |prj, cmd| {
    prj.add_test(
        "Fuzz.t.sol",
        r#"
import "forge-std/Test.sol";

contract FuzzTest is Test {
    constructor() {
        emit log("constructor");
    }

    function setUp() public {
        emit log("setUp");
    }

    function testShouldFailFuzz(uint8 x) public {
        emit log("testFailFuzz");
        require(x > 128, "should revert");
    }

    function testSuccessfulFuzz(uint128 a, uint128 b) public {
        emit log("testSuccessfulFuzz");
        assertEq(uint256(a) + uint256(b), uint256(a) + uint256(b));
    }

    function testToStringFuzz(bytes32 data) public {
        vm.toString(data);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 3 tests for test/Fuzz.t.sol:FuzzTest
[FAIL: should revert; counterexample: calldata=[..] args=[..]] testShouldFailFuzz(uint8) (runs: [..], [AVG_GAS])
[PASS] testSuccessfulFuzz(uint128,uint128) (runs: 256, [AVG_GAS])
[PASS] testToStringFuzz(bytes32) (runs: 256, [AVG_GAS])
Suite result: FAILED. 2 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 1 failed, 0 skipped (3 total tests)

Failing tests:
Encountered 1 failing test in test/Fuzz.t.sol:FuzzTest
[FAIL: should revert; counterexample: calldata=[..] args=[..]] testShouldFailFuzz(uint8) (runs: [..], [AVG_GAS])

Encountered a total of 1 failing tests, 2 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
});

// Test that showcases PUSH collection on normal fuzzing.
// Ignored until we collect them in a smarter way.
forgetest_init!(
    #[ignore]
    fuzz_collection,
    |prj, cmd| {
        prj.update_config(|config| {
            config.invariant.depth = 100;
            config.invariant.runs = 1000;
            config.fuzz.runs = 1000;
            config.fuzz.seed = Some(U256::from(6u32));
        });
        prj.add_test(
            "FuzzCollection.t.sol",
            r#"
import "forge-std/Test.sol";

contract SampleContract {
    uint256 public counter;
    uint256 public counterX2;
    address public owner = address(0xBEEF);
    bool public found_needle;

    event Incremented(uint256 counter);

    modifier onlyOwner() {
        require(msg.sender == owner, "ONLY_OWNER");
        _;
    }

    function compare(uint256 val) public {
        if (val == 0x4446) {
            found_needle = true;
        }
    }

    function incrementBy(uint256 numToIncrement) public onlyOwner {
        counter += numToIncrement;
        counterX2 += numToIncrement * 2;

        emit Incremented(counter);
    }

    function breakTheInvariant(uint256 x) public {
        if (x == 0x5556) {
            counterX2 = 0;
        }
    }
}

contract SampleContractTest is Test {
    event Incremented(uint256 counter);

    SampleContract public sample;

    function setUp() public {
        sample = new SampleContract();
    }

    function testIncrement(address caller) public {
        vm.startPrank(address(caller));

        vm.expectRevert("ONLY_OWNER");
        sample.incrementBy(1);
    }

    function testNeedle(uint256 needle) public {
        sample.compare(needle);
        require(!sample.found_needle(), "needle found.");
    }

    function invariantCounter() public {
        require(sample.counter() * 2 == sample.counterX2(), "broken counter.");
    }
}
   "#,
        );

        cmd.args(["test"]).assert_failure().stdout_eq(str![[r#""#]]);
    }
);

forgetest_init!(fuzz_failure_persist, |prj, cmd| {
    let persist_dir = prj.cache().parent().unwrap().join("persist");
    assert!(!persist_dir.exists());
    prj.update_config(|config| {
        config.fuzz.failure_persist_dir = Some(persist_dir.clone());
    });

    prj.add_test(
        "FuzzFailurePersist.t.sol",
        r#"
import "forge-std/Test.sol";

struct TestTuple {
    address user;
    uint256 amount;
}

contract FuzzFailurePersistTest is Test {
    function test_persist_fuzzed_failure(
        uint256 x,
        int256 y,
        address addr,
        bool cond,
        string calldata test,
        TestTuple calldata tuple,
        address[] calldata addresses
    ) public {
        // dummy assume to trigger runs
        vm.assume(x > 1 && x < 1111111111111111111111111111);
        vm.assume(y > 1 && y < 1111111111111111111111111111);
        require(false);
    }
}
   "#,
    );

    let mut calldata = None;
    let expected = str![[r#"
...
Ran 1 test for test/FuzzFailurePersist.t.sol:FuzzFailurePersistTest
[FAIL: EvmError: Revert; counterexample: calldata=[..] args=[..]] test_persist_fuzzed_failure(uint256,int256,address,bool,string,(address,uint256),address[]) (runs: 0, [AVG_GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]];
    let mut check = |cmd: &mut TestCommand, same: bool| {
        let assert = cmd.assert_failure();
        let output = assert.get_output();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let calldata = calldata.get_or_insert_with(|| {
            let re = Regex::new(r"calldata=(0x[0-9a-fA-F]+)").unwrap();
            re.captures(&stdout).unwrap().get(1).unwrap().as_str().to_string()
        });
        assert_eq!(stdout.contains(calldata.as_str()), same, "\n{stdout}");
        assert.stdout_eq(expected.clone());
    };

    cmd.args(["test", "-j1"]);

    // Run several times, asserting that the failure persists and is the same.
    for _ in 0..3 {
        check(&mut cmd, true);
        assert!(persist_dir.exists());
    }

    // Change dir and run again, asserting that the failure persists. It should be a new failure.
    let new_persist_dir = prj.cache().parent().unwrap().join("persist2");
    assert!(!new_persist_dir.exists());
    prj.update_config(|config| {
        config.fuzz.failure_persist_dir = Some(new_persist_dir.clone());
    });
    check(&mut cmd, false);
    assert!(new_persist_dir.exists());
});

// https://github.com/foundry-rs/foundry/pull/735 behavior changed with https://github.com/foundry-rs/foundry/issues/3521
// random values (instead edge cases) are generated if no fixtures defined
forgetest_init!(fuzz_int, |prj, cmd| {
    prj.add_test(
        "FuzzInt.t.sol",
        r#"
import "forge-std/Test.sol";

contract FuzzNumbersTest is Test {
    function testPositive(int256) public {
        assertTrue(true);
    }

    function testNegativeHalf(int256 val) public {
        assertTrue(val < 2 ** 128 - 1);
    }

    function testNegative0(int256 val) public {
        assertTrue(val == 0);
    }

    function testNegative1(int256 val) public {
        assertTrue(val == -1);
    }

    function testNegative2(int128 val) public {
        assertTrue(val == 1);
    }

    function testNegativeMax0(int256 val) public {
        assertTrue(val == type(int256).max);
    }

    function testNegativeMax1(int256 val) public {
        assertTrue(val == type(int256).max - 2);
    }

    function testNegativeMin0(int256 val) public {
        assertTrue(val == type(int256).min);
    }

    function testNegativeMin1(int256 val) public {
        assertTrue(val == type(int256).min + 2);
    }

    function testEquality(int256 x, int256 y) public {
        int256 xy;

        unchecked {
            xy = x * y;
        }

        if ((x != 0 && xy / x != y)) {
            return;
        }

        assertEq(((xy - 1) / 1e18) + 1, (xy - 1) / (1e18 + 1));
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 10 tests for test/FuzzInt.t.sol:FuzzNumbersTest
[FAIL: assertion failed[..]] testEquality(int256,int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative0(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative1(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative2(int128) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeHalf(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMax0(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMax1(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMin0(int256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMin1(int256) (runs: [..], [AVG_GAS])
[PASS] testPositive(int256) (runs: 256, [AVG_GAS])
Suite result: FAILED. 1 passed; 9 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 9 failed, 0 skipped (10 total tests)
...
"#]]);
});

forgetest_init!(fuzz_positive, |prj, cmd| {
    prj.add_test(
        "FuzzPositive.t.sol",
        r#"
import "forge-std/Test.sol";

contract FuzzPositive is Test {
    function testSuccessChecker(uint256 val) public {
        assertTrue(true);
    }

    function testSuccessChecker2(int256 val) public {
        assert(val == val);
    }

    function testSuccessChecker3(uint32 val) public {
        assert(val + 0 == val);
    }
}
   "#,
    );

    cmd.args(["test"]).assert_success().stdout_eq(str![[r#"
...
Ran 3 tests for test/FuzzPositive.t.sol:FuzzPositive
[PASS] testSuccessChecker(uint256) (runs: 256, [AVG_GAS])
[PASS] testSuccessChecker2(int256) (runs: 256, [AVG_GAS])
[PASS] testSuccessChecker3(uint32) (runs: 256, [AVG_GAS])
Suite result: ok. 3 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 3 tests passed, 0 failed, 0 skipped (3 total tests)

"#]]);
});

// https://github.com/foundry-rs/foundry/pull/735 behavior changed with https://github.com/foundry-rs/foundry/issues/3521
// random values (instead edge cases) are generated if no fixtures defined
forgetest_init!(fuzz_uint, |prj, cmd| {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(100u32));
    });
    prj.add_test(
        "FuzzUint.t.sol",
        r#"
import "forge-std/Test.sol";

contract FuzzNumbersTest is Test {
    function testPositive(uint256) public {
        assertTrue(true);
    }

    function testNegativeHalf(uint256 val) public {
        assertTrue(val < 2 ** 128 - 1);
    }

    function testNegative0(uint256 val) public {
        assertTrue(val == 0);
    }

    function testNegative2(uint256 val) public {
        assertTrue(val == 2);
    }

    function testNegative2Max(uint256 val) public {
        assertTrue(val == type(uint256).max - 2);
    }

    function testNegativeMax(uint256 val) public {
        assertTrue(val == type(uint256).max);
    }

    function testEquality(uint256 x, uint256 y) public {
        uint256 xy;

        unchecked {
            xy = x * y;
        }

        if ((x != 0 && xy / x != y)) {
            return;
        }

        assertEq(((xy - 1) / 1e18) + 1, (xy - 1) / (1e18 + 1));
    }
}
   "#,
    );

    cmd.args(["test"]).assert_failure().stdout_eq(str![[r#"
...
Ran 7 tests for test/FuzzUint.t.sol:FuzzNumbersTest
[FAIL: assertion failed[..]] testEquality(uint256,uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative0(uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative2(uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegative2Max(uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeHalf(uint256) (runs: [..], [AVG_GAS])
[FAIL: assertion failed[..]] testNegativeMax(uint256) (runs: [..], [AVG_GAS])
[PASS] testPositive(uint256) (runs: 256, [AVG_GAS])
Suite result: FAILED. 1 passed; 6 failed; 0 skipped; [ELAPSED]
...
"#]]);
});

forgetest_init!(should_fuzz_literals, |prj, cmd| {
    // Add a source with magic (literal) values
    prj.add_source(
        "Magic.sol",
        r#"
        contract Magic {
            // plain literals
            address constant DAI = 0x6B175474E89094C44Da98b954EedeAC495271d0F;
            uint64 constant MAGIC_NUMBER = 1122334455;
            int32 constant MAGIC_INT = -777;
            bytes32 constant MAGIC_WORD = "abcd1234";
            bytes constant MAGIC_BYTES = hex"deadbeef";
            string constant MAGIC_STRING = "xyzzy";

            function checkAddr(address v) external pure { assert(v != DAI); }
            function checkWord(bytes32 v) external pure { assert(v != MAGIC_WORD); }
            function checkNumber(uint64 v) external pure { assert(v != MAGIC_NUMBER); }
            function checkInteger(int32 v) external pure { assert(v != MAGIC_INT); }
            function checkString(string memory v) external pure { assert(keccak256(abi.encodePacked(v)) != keccak256(abi.encodePacked(MAGIC_STRING))); }
            function checkBytesFromHex(bytes memory v) external pure { assert(keccak256(v) != keccak256(MAGIC_BYTES)); }
            function checkBytesFromString(bytes memory v) external pure { assert(keccak256(v) != keccak256(abi.encodePacked(MAGIC_STRING))); }
        }
        "#,
    );

    prj.add_test(
        "MagicFuzz.t.sol",
        r#"
            import {Test} from "forge-std/Test.sol";
            import {Magic} from "src/Magic.sol";

            contract MagicTest is Test {
                Magic public magic;
                function setUp() public { magic = new Magic(); }

                function testFuzz_Addr(address v) public view { magic.checkAddr(v); }
                function testFuzz_Number(uint64 v) public view { magic.checkNumber(v); }
                function testFuzz_Integer(int32 v) public view { magic.checkInteger(v); }
                function testFuzz_Word(bytes32 v) public view { magic.checkWord(v); }
                function testFuzz_String(string memory v) public view { magic.checkString(v); }
                function testFuzz_BytesFromHex(bytes memory v) public view { magic.checkBytesFromHex(v); }
                function testFuzz_BytesFromString(bytes memory v) public view { magic.checkBytesFromString(v); }
            }
        "#,
    );

    // Helper to create expected output for a test failure
    let expected_fail = |test_name: &str, type_sig: &str, value: &str| -> String {
        format!(
            r#"No files changed, compilation skipped

Ran 1 test for test/MagicFuzz.t.sol:MagicTest
[FAIL: panic: assertion failed (0x01); counterexample: calldata=[..] args=[{value}]] {test_name}({type_sig}) (runs: [..], [AVG_GAS])
[..]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
...
Encountered a total of 1 failing tests, 0 tests succeeded
...
"#
        )
    };

    // Test address literal fuzzing
    let mut test_literal = |seed: u32,
                            test_name: &'static str,
                            type_sig: &'static str,
                            expected_value: &'static str| {
        // the fuzzer is UNABLE to find a breaking input (fast) when NOT seeding from the AST
        prj.update_config(|config| {
            config.fuzz.runs = 100;
            config.fuzz.dictionary.max_fuzz_dictionary_literals = 0;
            config.fuzz.seed = Some(U256::from(seed));
        });
        cmd.forge_fuse().args(["test", "--match-test", test_name, "-j1"]).assert_success();

        // the fuzzer is ABLE to find a breaking input when seeding from the AST
        prj.update_config(|config| {
            config.fuzz.dictionary.max_fuzz_dictionary_literals = 10_000;
        });

        let expected_output = expected_fail(test_name, type_sig, expected_value);
        cmd.forge_fuse()
            .args(["test", "--match-test", test_name, "-j1"])
            .assert_failure()
            .stdout_eq(expected_output);
    };

    test_literal(100, "testFuzz_Addr", "address", "0x6B175474E89094C44Da98b954EedeAC495271d0F");
    test_literal(200, "testFuzz_Number", "uint64", "1122334455 [1.122e9]");
    test_literal(300, "testFuzz_Integer", "int32", "-777");
    test_literal(
        400,
        "testFuzz_Word",
        "bytes32",
        "0x6162636431323334000000000000000000000000000000000000000000000000", /* bytes32("abcd1234") */
    );
    test_literal(500, "testFuzz_BytesFromHex", "bytes", "0xdeadbeef");
    test_literal(600, "testFuzz_String", "string", "\"xyzzy\"");
    test_literal(999, "testFuzz_BytesFromString", "bytes", "0x78797a7a79"); // abi.encodePacked("xyzzy")
});

// Tests that `vm.randomUint()` produces different values across fuzz runs.
// Regression test for https://github.com/foundry-rs/foundry/issues/12817
//
// The issue was that `vm.randomUint()` would produce the same sequence of values
// in every fuzz run because the RNG was seeded identically for each run.
// This test verifies that with many fuzz runs and a small range, we eventually
// hit value 0, which proves the RNG varies across runs.
forgetest_init!(test_fuzz_random_uint_varies_across_runs, |prj, cmd| {
    prj.add_test(
        "RandomFuzzTest.t.sol",
        r#"
pragma solidity >=0.8.0;

import {Test} from "forge-std/Test.sol";

contract RandomFuzzTest is Test {
    function testFuzz_randomUint_shouldFail(uint256) public {
        uint256 rand = vm.randomUint(0, 4);
        assertTrue(rand != 0, "hit value 0");
    }
}
   "#,
    );

    cmd.args(["test", "--fuzz-seed", "1", "--mt", "testFuzz_randomUint_shouldFail"])
        .assert_failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/RandomFuzzTest.t.sol:RandomFuzzTest
[FAIL: hit value 0; counterexample: [..]] testFuzz_randomUint_shouldFail(uint256) (runs: [..], [AVG_GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)
...
"#]]);
});

forgetest_init!(test_fuzz_run_replays_random_uint_failure, |prj, cmd| {
    prj.add_test(
        "RandomFuzzTest.t.sol",
        r#"
pragma solidity >=0.8.0;

import {Test} from "forge-std/Test.sol";

contract RandomFuzzTest is Test {
    function testFuzz_randomUint_shouldFail(uint256) public {
        uint256 rand = vm.randomUint(0, 4);
        assertTrue(rand != 0, "hit value 0");
    }
}
   "#,
    );

    let expected_output = str![[r#"
...
Ran 1 test for test/RandomFuzzTest.t.sol:RandomFuzzTest
[FAIL: hit value 0; counterexample: [..]] testFuzz_randomUint_shouldFail(uint256) (runs: [..], [AVG_GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]];

    cmd.args(["test", "--fuzz-seed", "1", "--mt", "testFuzz_randomUint_shouldFail", "-j1"])
        .assert_failure()
        .stdout_eq(expected_output.clone());

    let failure_file =
        prj.root().join("cache/fuzz/failures/RandomFuzzTest/testFuzz_randomUint_shouldFail");
    let persisted_failure: BaseCounterExample =
        serde_json::from_slice(&std::fs::read(&failure_file).unwrap()).unwrap();
    assert_eq!(persisted_failure.fuzz.seed, Some(U256::from(1)));
    assert_eq!(persisted_failure.fuzz.worker, Some(0));
    let fuzz_run = persisted_failure.fuzz.run.unwrap().to_string();
    let fuzz_worker = persisted_failure.fuzz.worker.unwrap().to_string();

    cmd.forge_fuse()
        .args([
            "test",
            "--fuzz-seed",
            "1",
            "--fuzz-run",
            &fuzz_run,
            "--fuzz-worker",
            &fuzz_worker,
            "--mt",
            "testFuzz_randomUint_shouldFail",
            "-j1",
        ])
        .assert_failure()
        .stdout_eq(expected_output.clone());

    cmd.forge_fuse().args(["test", "--rerun", "-j1"]).assert_failure().stdout_eq(expected_output);
});

forgetest_init!(test_fuzz_rerun_replays_random_uint_failure_without_seed, |prj, cmd| {
    prj.add_test(
        "RandomFuzzTest.t.sol",
        r#"
pragma solidity >=0.8.0;

import {Test} from "forge-std/Test.sol";

contract RandomFuzzTest is Test {
    error Random(uint256 value);

    function testFuzz_randomUint_shouldFail(uint256) public {
        revert Random(vm.randomUint());
    }
}
   "#,
    );

    let expected_output = str![[r#"
...
Ran 1 test for test/RandomFuzzTest.t.sol:RandomFuzzTest
[FAIL: Random([..]); counterexample: [..]] testFuzz_randomUint_shouldFail(uint256) (runs: [..], [AVG_GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

[SEED] (use `--fuzz-seed` to reproduce)

"#]];

    let assert = cmd
        .args(["test", "--mt", "testFuzz_randomUint_shouldFail", "-j1"])
        .assert_failure()
        .stdout_eq(expected_output.clone());
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let reason = random_failure_reason(&stdout);

    let failure_file =
        prj.root().join("cache/fuzz/failures/RandomFuzzTest/testFuzz_randomUint_shouldFail");
    let persisted_failure: BaseCounterExample =
        serde_json::from_slice(&std::fs::read(&failure_file).unwrap()).unwrap();
    let fuzz_seed = format!("{:#x}", persisted_failure.fuzz.seed.unwrap());
    let fuzz_run = persisted_failure.fuzz.run.unwrap().to_string();
    let fuzz_worker = persisted_failure.fuzz.worker.unwrap().to_string();

    let assert = cmd
        .forge_fuse()
        .args([
            "test",
            "--fuzz-seed",
            &fuzz_seed,
            "--fuzz-run",
            &fuzz_run,
            "--fuzz-worker",
            &fuzz_worker,
            "--mt",
            "testFuzz_randomUint_shouldFail",
            "-j1",
        ])
        .assert_failure()
        .stdout_eq(expected_output.clone());
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert_eq!(random_failure_reason(&stdout), reason, "{stdout}");

    let assert = cmd
        .forge_fuse()
        .args(["test", "--rerun", "-j1"])
        .assert_failure()
        .stdout_eq(expected_output);
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert_eq!(random_failure_reason(&stdout), reason, "{stdout}");

    let assert = cmd.forge_fuse().args(["test", "--rerun", "-j1"]).assert_failure();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert_eq!(random_failure_reason(&stdout), reason, "{stdout}");
});

// Fuzzed enum inputs must stay within `0..variant_count`, else the contract rejects them with
// `Panic(0x21)` when decoding, before the test body runs. https://github.com/foundry-rs/foundry/issues/6623
forgetest_init!(fuzz_bounds_enum_inputs, |prj, cmd| {
    // File-level enum in a non-test source, used as a struct field below, to exercise enum
    // collection from outside the test file.
    prj.add_source(
        "LibEnum.sol",
        r#"
enum LibEnumVal { L0, L1 }
   "#,
    );

    prj.add_test(
        "FuzzEnum.t.sol",
        r#"
import "forge-std/Test.sol";
import {LibEnumVal} from "src/LibEnum.sol";

contract FuzzEnum is Test {
    enum EnumVal { VAL_0, VAL_1, VAL_2 }

    struct WithEnum {
        EnumVal e;
        uint8 raw;
        LibEnumVal lib;
    }

    function testScalarEnum(EnumVal val) public pure {
        assert(uint8(val) < 3);
    }

    function testEnumArray(EnumVal[] memory vals) public pure {
        for (uint256 i; i < vals.length; ++i) {
            assert(uint8(vals[i]) < 3);
        }
    }

    function testEnumInStruct(WithEnum memory s) public pure {
        assert(uint8(s.e) < 3);
        assert(uint8(s.lib) < 2);
    }

    function testEnumInStructArray(WithEnum[] memory xs) public pure {
        for (uint256 i; i < xs.length; ++i) {
            assert(uint8(xs[i].e) < 3);
            assert(uint8(xs[i].lib) < 2);
        }
    }
}
   "#,
    );

    cmd.args(["test", "--mc", "FuzzEnum", "--fuzz-runs", "512"]).assert_success().stdout_eq(str![
        [r#"
...
Ran 4 tests for test/FuzzEnum.t.sol:FuzzEnum
[PASS] testEnumArray(uint8[]) (runs: 512, [AVG_GAS])
[PASS] testEnumInStruct((uint8,uint8,uint8)) (runs: 512, [AVG_GAS])
[PASS] testEnumInStructArray((uint8,uint8,uint8)[]) (runs: 512, [AVG_GAS])
[PASS] testScalarEnum(uint8) (runs: 512, [AVG_GAS])
Suite result: ok. 4 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 4 tests passed, 0 failed, 0 skipped (4 total tests)

"#]
    ]);
});

fn random_failure_reason(stdout: &str) -> String {
    Regex::new(r"\[FAIL: (Random\([^)]+\))")
        .unwrap()
        .captures(stdout)
        .unwrap_or_else(|| panic!("{stdout}"))[1]
        .to_string()
}
