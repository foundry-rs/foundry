use alloy_json_abi::JsonAbi;
use alloy_primitives::{U256, hex};
use foundry_evm::fuzz::BaseCounterExample;
use foundry_test_utils::{TestCommand, forgetest_init, str};
use regex::Regex;
use serde_json::Value;

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

    cmd.forge_fuse()
        .args(["fuzz", "replay", "--mc", "ForgeFuzzReplayFailureTest", "-vvv"])
        .assert_failure()
        .stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 2 tests for test/ForgeFuzzReplayFailure.t.sol:ForgeFuzzReplayFailureTest
[FAIL: EvmError: Revert; counterexample: calldata=0x[..] args=[200]] testFuzz_reverts(uint256) (runs: 0, [AVG_GAS])
Traces:
  [[..]] ForgeFuzzReplayFailureTest::testFuzz_reverts(200)
    └─ ← [Revert] EvmError: Revert

Backtrace:
  at ForgeFuzzReplayFailureTest.testFuzz_reverts

[SKIP: not runnable in replay mode] test_unit() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 1 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 1 skipped (2 total tests)

Failing tests:
Encountered 1 failing test in test/ForgeFuzzReplayFailure.t.sol:ForgeFuzzReplayFailureTest
[FAIL: EvmError: Revert; counterexample: calldata=0x[..] args=[200]] testFuzz_reverts(uint256) (runs: 0, [AVG_GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
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

forgetest_init!(forge_fuzz_show_cmin_tmin_corpus_files, |prj, cmd| {
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
    let entry = r#"[{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"0x938872f7000000000000000000000000000000000000000000000000000000000000002a",
  "value":"0x0"
}]"#;
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000001-1.json"), entry).unwrap();
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000002-2.json"), entry).unwrap();

    cmd.forge_fuse()
        .args(["fuzz", "show", "corpus"])
        .assert_success()
        .stdout_eq(str![[r#"
corpus/00000000-0000-0000-0000-000000000001-1.json (1 txs)
  0: ForgeFuzzShowTargetTest.testFuzz_setNumber(42) sender=0x0000000000000000000000000000000000000001 target=0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496 value=0
corpus/00000000-0000-0000-0000-000000000002-2.json (1 txs)
  0: ForgeFuzzShowTargetTest.testFuzz_setNumber(42) sender=0x0000000000000000000000000000000000000001 target=0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496 value=0

"#]]);

    cmd.forge_fuse()
        .args(["fuzz", "cmin", "--mc", "ForgeFuzzShowTargetTest", "corpus", "--out", "cmin"])
        .assert_success()
        .stdout_eq(str![[r#"
minimized corpus: kept 1/2 entries in cmin

"#]]);
    let cmin_entries = std::fs::read_dir(prj.root().join("cmin")).unwrap().count();
    assert_eq!(cmin_entries, 1);

    cmd.forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzShowTargetTest",
            "corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--out",
            "cmin-file",
        ])
        .assert_success()
        .stdout_eq(str![[r#"
minimized corpus: kept 1/1 entries in cmin-file

"#]]);
    assert!(prj.root().join("cmin-file/00000000-0000-0000-0000-000000000001-1.json").is_file());

    let tmin = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzShowTargetTest",
            "corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--out",
            "min.json",
        ])
        .assert_success();
    let stdout = String::from_utf8(tmin.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("minimized entry: 1 txs -> min.json"), "{stdout}");
    assert!(prj.root().join("min.json").is_file());

    let show_min = cmd.forge_fuse().args(["fuzz", "show", "min.json"]).assert_success();
    let stdout = String::from_utf8(show_min.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("min.json (1 txs)"), "{stdout}");
    assert!(stdout.contains("ForgeFuzzShowTargetTest.testFuzz_setNumber("), "{stdout}");

    let replay = cmd
        .forge_fuse()
        .args(["fuzz", "replay", "--mc", "ForgeFuzzShowTargetTest", "--corpus-dir", "min.json"])
        .assert_success();
    let stdout = String::from_utf8(replay.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[PASS] testFuzz_setNumber(uint256) (replay: 1 entries"), "{stdout}");
});

forgetest_init!(forge_fuzz_cmin_keeps_coverage_divergent_entries, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzCminCoverage.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzCminCoverageTest is Test {
    uint256 public sink;

    function testFuzz_branch(uint256 value) public {
        if (value == 1) {
            sink = 1;
        } else if (value == 2) {
            sink = 2;
        } else {
            sink = 3;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let entry_one = r#"[{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"0x003919a00000000000000000000000000000000000000000000000000000000000000001",
  "value":"0x0"
}]"#;
    let entry_two = r#"[{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"0x003919a00000000000000000000000000000000000000000000000000000000000000002",
  "value":"0x0"
}]"#;
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000001-1.json"), entry_one).unwrap();
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000002-2.json"), entry_two).unwrap();

    let cmin = cmd
        .forge_fuse()
        .args(["fuzz", "cmin", "--mc", "ForgeFuzzCminCoverageTest", "corpus", "--out", "cmin"])
        .assert_success();
    let stdout = String::from_utf8(cmin.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("minimized corpus: kept 2/2 entries in cmin"), "{stdout}");
    assert_eq!(std::fs::read_dir(prj.root().join("cmin")).unwrap().count(), 2);
});

forgetest_init!(forge_fuzz_tmin_reuses_session_across_candidates, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminSession.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzTminSessionTest is Test {
    uint256 public sink;

    function testFuzz_branch(uint256 value) public {
        if (value == 1) {
            sink = 1;
        } else if (value == 2) {
            sink = 2;
        } else {
            sink = 3;
        }
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let entry = r#"[{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"0x003919a00000000000000000000000000000000000000000000000000000000000000001",
  "value":"0x0"
},{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"0x003919a00000000000000000000000000000000000000000000000000000000000000002",
  "value":"0x0"
},{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"0x003919a0000000000000000000000000000000000000000000000000000000000000002a",
  "value":"0x0"
}]"#;
    std::fs::write(corpus.join("multi.json"), entry).unwrap();

    let tmin = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminSessionTest",
            "corpus/multi.json",
            "--out",
            "min-session.json",
            "--max-attempts",
            "4",
        ])
        .assert_success();
    let stdout = String::from_utf8(tmin.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("after 4 candidate replays"), "{stdout}");
    assert!(prj.root().join("min-session.json").is_file());
});

forgetest_init!(forge_fuzz_tmin_preserves_revert_data, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzTminReason.t.sol",
        r#"
contract ForgeFuzzTminReasonTest {
    function testFuzz_reason(uint256 value) public pure {
        if (value == 0) revert("zero");
        if (value == 2) revert("two");
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let artifact = std::fs::read_to_string(
        prj.root().join("out/ForgeFuzzTminReason.t.sol/ForgeFuzzTminReasonTest.json"),
    )
    .unwrap();
    let artifact: Value = serde_json::from_str(&artifact).unwrap();
    let abi: JsonAbi = serde_json::from_value(artifact["abi"].clone()).unwrap();
    let function = abi.functions().find(|function| function.name == "testFuzz_reason").unwrap();
    let calldata_two = format!("0x{}{:064x}", hex::encode(function.selector()), 2);
    let entry = format!(
        r#"[{{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"{calldata_two}",
  "value":"0x0"
}}]"#
    );
    std::fs::write(corpus.join("reason.json"), entry).unwrap();

    let tmin = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzTminReasonTest",
            "corpus/reason.json",
            "--out",
            "min-reason.json",
        ])
        .assert_success();
    let stdout = String::from_utf8(tmin.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("preserved original failure"), "{stdout}");

    let min = std::fs::read_to_string(prj.root().join("min-reason.json")).unwrap();
    assert!(min.contains(&calldata_two), "{min}");
});

forgetest_init!(forge_fuzz_replay_invariant_fail_on_revert, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 1;
        config.invariant.fail_on_revert = false;
        config.invariant.corpus.corpus_dir = Some("invariant_corpus".into());
        config.invariant.corpus.corpus_gzip = false;
    });
    prj.add_test(
        "ForgeFuzzInvariantFailOnRevertReplay.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";

contract ForgeFuzzInvariantFailOnRevertReplayTest is Test {
    function setUp() public {
        targetContract(address(this));
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = this.revertHandler.selector;
        targetSelector(FuzzSelector({addr: address(this), selectors: selectors}));
    }

    function revertHandler(uint256 value) external pure {
        value;
        revert("boom");
    }

    function invariant_ok() public view {}
}
   "#,
    );
    cmd.args([
        "test",
        "--mc",
        "ForgeFuzzInvariantFailOnRevertReplayTest",
        "--mt",
        "invariant_ok",
        "-q",
    ])
    .assert_success();

    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
    });
    let corpus_entry = find_first_json(&prj.root().join("invariant_corpus"));
    let corpus_entry = corpus_entry.strip_prefix(prj.root()).unwrap().to_str().unwrap().to_string();

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
    assert!(stdout.contains("failed during replay: handler:"), "{stdout}");

    let tmin = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzInvariantFailOnRevertReplayTest",
            "--mt",
            "invariant_ok",
            &corpus_entry,
            "--out",
            "min-fail-on-revert.json",
        ])
        .assert_success();
    let stdout = String::from_utf8(tmin.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("preserved original failure"), "{stdout}");

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
    let artifact = std::fs::read_to_string(prj.root().join(
        "out/ForgeFuzzInvariantReplaySequence.t.sol/ForgeFuzzInvariantReplaySequenceTest.json",
    ))
    .unwrap();
    let artifact: Value = serde_json::from_str(&artifact).unwrap();
    let abi: JsonAbi = serde_json::from_value(artifact["abi"].clone()).unwrap();
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
    assert!(stdout.contains("broke invariant during replay: afterInvariant:"), "{stdout}");
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
    assert_eq!(stdout.matches("corpus/worker").count(), 1, "{stdout}");
});

forgetest_init!(forge_fuzz_rejects_machine, |prj, cmd| {
    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let entry = r#"[{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x0000000000000000000000000000000000000002",
  "calldata":"0x12345678",
  "value":"0x0"
}]"#;
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000001-1.json"), entry).unwrap();
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000002-2.json"), entry).unwrap();

    for args in [
        vec!["--machine", "fuzz", "run"],
        vec!["--machine", "fuzz", "replay"],
        vec!["--machine", "fuzz", "show", "corpus"],
        vec!["--machine", "fuzz", "cmin", "corpus", "--out", "cmin"],
        vec![
            "--machine",
            "fuzz",
            "tmin",
            "corpus/00000000-0000-0000-0000-000000000001-1.json",
            "--out",
            "min-machine.json",
        ],
    ] {
        let result = cmd.forge_fuse().args(args).assert_failure();
        let output: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
        assert_eq!(output["success"], false);
        assert_eq!(output["errors"][0]["code"], "cli.usage.invalid");
        assert_eq!(output["errors"][0]["details"]["subcommand"], "fuzz");
    }
});

forgetest_init!(forge_fuzz_cmin_tmin_error_on_zero_replay, |prj, cmd| {
    prj.add_test(
        "ForgeFuzzZeroReplay.t.sol",
        r#"
contract ForgeFuzzZeroReplayTest {
    function testFuzz_branch(uint256 value) public pure {
        value;
    }
}
   "#,
    );
    cmd.args(["build", "-q"]).assert_success();

    let corpus = prj.root().join("corpus");
    std::fs::create_dir_all(&corpus).unwrap();
    let entry = r#"[{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"0x003919a00000000000000000000000000000000000000000000000000000000000000001",
  "value":"0x0"
}]"#;
    std::fs::write(corpus.join("00000000-0000-0000-0000-000000000001-1.json"), entry).unwrap();

    let cmin = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzZeroReplayTest",
            "--mt",
            "testFuzz_noMatch",
            "corpus",
            "--out",
            "cmin",
        ])
        .assert_failure();
    let stderr = String::from_utf8(cmin.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains(
            "fuzz minimization requires exactly one matched fuzz or invariant test; matched 0"
        ),
        "{stderr}"
    );
    assert!(!prj.root().join("cmin").exists());

    let wrong_corpus = prj.root().join("wrong-corpus");
    std::fs::create_dir_all(&wrong_corpus).unwrap();
    let wrong_entry = r#"[{
  "sender":"0x0000000000000000000000000000000000000001",
  "target":"0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
  "calldata":"0xdeadbeef0000000000000000000000000000000000000000000000000000000000000001",
  "value":"0x0"
}]"#;
    std::fs::write(wrong_corpus.join("00000000-0000-0000-0000-000000000002-2.json"), wrong_entry)
        .unwrap();

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

    let zero_replay_cmin = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzZeroReplayTest",
            "--mt",
            "testFuzz_branch",
            "wrong-corpus",
            "--out",
            "zero-replay-cmin",
        ])
        .assert_failure();
    let stderr = String::from_utf8(zero_replay_cmin.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("replayed 0 transactions from wrong-corpus"), "{stderr}");
    assert!(!prj.root().join("zero-replay-cmin").exists());

    cmd.forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzZeroReplayTest",
            "--mt",
            "testFuzz_branch",
            "corpus",
            "--out",
            "cmin",
        ])
        .assert_success();
    assert!(prj.root().join("cmin/00000000-0000-0000-0000-000000000001-1.json").is_file());

    let tmin = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "tmin",
            "--mc",
            "ForgeFuzzZeroReplayTest",
            "--mt",
            "testFuzz_branch",
            "wrong-corpus/00000000-0000-0000-0000-000000000002-2.json",
            "--out",
            "min.json",
        ])
        .assert_failure();
    let stderr = String::from_utf8(tmin.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains(
            "replayed 0 transactions from wrong-corpus/00000000-0000-0000-0000-000000000002-2.json"
        ),
        "{stderr}"
    );
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
    assert!(
        show_stdout.contains("fuzz_corpus/ForgeFuzzGeneratedCorpusTest/testFuzz_SetNumber"),
        "{show_stdout}"
    );

    let cmin = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "cmin",
            "--mc",
            "ForgeFuzzGeneratedCorpusTest",
            "--mt",
            "testFuzz_SetNumber",
            "fuzz_corpus",
            "--out",
            "cmin-root",
        ])
        .assert_success();
    let cmin_stdout = String::from_utf8(cmin.get_output().stdout.clone()).unwrap();
    assert!(!cmin_stdout.contains("kept 0/0 entries"), "{cmin_stdout}");
    assert!(std::fs::read_dir(prj.root().join("cmin-root")).unwrap().count() > 0);

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
    assert!(
        invariant_stdout.contains("invariant_corpus/ForgeFuzzGeneratedCorpusTest/worker0/corpus"),
        "{invariant_stdout}"
    );
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

"#]]);
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

fn random_failure_reason(stdout: &str) -> String {
    Regex::new(r"\[FAIL: (Random\([^)]+\))")
        .unwrap()
        .captures(stdout)
        .unwrap_or_else(|| panic!("{stdout}"))[1]
        .to_string()
}
