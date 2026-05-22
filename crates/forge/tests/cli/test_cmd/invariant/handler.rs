//! E2E tests for handler-side assertion bugs: dedup by `(reverter, selector)` site,
//! persistence under `failures/<contract>/handlers/<keccak256(reverter‖selector)>.json`,
//! replay/shrink semantics, and stale-file cleanup. Distinct from invariant predicate
//! failures.

use foundry_test_utils::{forgetest_init, str};

// Handler `assert(false)` surfaces under `Assertion Tests:`, not as a live invariant failure.
forgetest_init!(handler_assertion_routed_to_handler_section, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = false;
    });
    prj.add_source(
        "AssertHandler.sol",
        r#"
contract AssertHandler {
    uint256 public calls;

    function alwaysAssert() external {
        calls++;
        assert(false);
    }
}
   "#,
    );
    prj.add_test(
        "AssertAllAssertTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {AssertHandler} from "../src/AssertHandler.sol";

contract AssertAllAssertTest is Test {
    AssertHandler handler;

    function setUp() public {
        handler = new AssertHandler();
        targetContract(address(handler));
    }

    function invariant_a() public view {}

    function invariant_b() public view {}
}
   "#,
    );

    let output = cmd.args(["test", "--mt", "invariant_a"]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("Ran 1 test for test/AssertAllAssertTest.t.sol:AssertAllAssertTest"));
    assert!(stdout.contains("Assertion Tests: 1 assertion bug(s) found"), "{stdout}");
    assert!(
        stdout.contains(
            "[FAIL: panic: assertion failed (0x01)] src/AssertHandler.sol:AssertHandler::alwaysAssert"
        ),
        "{stdout}"
    );
    assert!(stdout.contains(" invariant_a() (runs: 1, calls:"), "{stdout}");
    assert!(!stdout.contains("Invariant/Property Tests:"), "{stdout}");
    assert!(!stdout.contains("invariant_b"), "{stdout}");
});

// Site-granular dedup: 4 distinct paths through one selector all share the same
// `(reverter, selector)` site → 1 persisted bug, shrunk to its anchor. Echidna/Medusa
// semantics: one bug per `(handler, function)`, regardless of which code path reached it.
// If the handler is patched to no longer assert, the persisted file is deleted on replay.
forgetest_init!(handler_assertion_dedupes_by_site, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 200;
        config.invariant.fail_on_revert = false;
        config.invariant.corpus.corpus_dir = Some("inv_corpus".into());
    });
    prj.add_source(
        "MultiPathHandler.sol",
        r#"
contract MultiPathHandler {
    uint256 public state;

    // Filler so fuzzing produces a prefix to shrink.
    function noop() external {}

    // Four branches, one selector — all asserting via the same `(reverter, selector)`
    // site. Under site-granular dedup these collapse into a single persisted bug
    // (Echidna/Medusa semantics).
    function maybeAssert(uint8 path) external {
        if (path < 64) {
            state = 1;
            assert(false);
        } else if (path < 128) {
            state = 2;
            assert(false);
        } else if (path < 192) {
            state = 3;
            assert(false);
        } else {
            state = 4;
            assert(false);
        }
    }
}
   "#,
    );
    prj.add_test(
        "MultiPathTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {MultiPathHandler} from "../src/MultiPathHandler.sol";

contract MultiPathTest is Test {
    MultiPathHandler handler;

    function setUp() public {
        handler = new MultiPathHandler();
        targetContract(address(handler));
    }

    function invariant_ok() public view {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_ok", "--fuzz-seed", "119"]).assert_failure().stdout_eq(
        str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/MultiPathTest.t.sol:MultiPathTest
Assertion Tests: 1 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/MultiPathHandler.sol:MultiPathHandler::maybeAssert
	[Sequence] (original: 2, shrunk: 1)
		sender=[..] addr=[src/MultiPathHandler.sol:MultiPathHandler]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=maybeAssert(uint8) args=[2]
 invariant_ok() (runs: 1, calls: 200, reverts: 102)

╭------------------+-------------+-------+---------+----------╮
| Contract         | Selector    | Calls | Reverts | Discards |
+=============================================================+
| MultiPathHandler | maybeAssert | 102   | 102     | 0        |
|------------------+-------------+-------+---------+----------|
| MultiPathHandler | noop        | 98    | 0       | 0        |
╰------------------+-------------+-------+---------+----------╯

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/MultiPathTest.t.sol:MultiPathTest
Assertion Tests: 1 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/MultiPathHandler.sol:MultiPathHandler::maybeAssert
	[Sequence] (original: 2, shrunk: 1)
		sender=[..] addr=[src/MultiPathHandler.sol:MultiPathHandler]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=maybeAssert(uint8) args=[2]
 invariant_ok() (runs: 1, calls: 200, reverts: 102)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]],
    );

    let handlers_dir = prj
        .root()
        .join("cache")
        .join("invariant")
        .join("failures")
        .join("MultiPathTest")
        .join("handlers");
    let before: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(
        before.len(),
        1,
        "expected 1 persisted handler bug after site-granular dedup, got {before:?}",
    );

    // Stale-file deletion: patch the handler so `maybeAssert` no longer asserts. On the
    // next replay the persisted reproducer's anchor stops asserting → the stale file is
    // deleted in place.
    prj.add_source(
        "MultiPathHandler.sol",
        r#"
contract MultiPathHandler {
    uint256 public state;
    function noop() external {}
    function maybeAssert(uint8 path) external { state = uint256(path) % 4 + 1; }
}
   "#,
    );
    prj.update_config(|config| {
        config.invariant.runs = 0;
    });
    // With `runs=0` no new fuzzing happens; the only work is replaying the persisted
    // reproducer, which now no longer asserts → file deleted → no handler bug → success.
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok", "--fuzz-seed", "119"]).assert_success();

    let after: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert!(after.is_empty(), "stale handler-bug file should be deleted, got {after:?}");
});

// Bug persists to `failures/<contract>/handlers/<fingerprint>.json`, replays from disk,
// and is deleted when stale (handler patched to no-op, or anchor stops asserting while the
// invariant breaks instead).
forgetest_init!(handler_assertion_persisted_to_disk, |prj, cmd| {
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
        "AlwaysAssertTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {AlwaysAssert} from "../src/AlwaysAssert.sol";

contract AlwaysAssertTest is Test {
    AlwaysAssert h;
    function setUp() public { h = new AlwaysAssert(); targetContract(address(h)); }
    function invariant_ok() public view {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_ok"]).assert_failure();

    let handlers_dir = prj
        .root()
        .join("cache")
        .join("invariant")
        .join("failures")
        .join("AlwaysAssertTest")
        .join("handlers");
    assert!(handlers_dir.exists(), "handlers dir not created: {handlers_dir:?}");
    let entries: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert!(!entries.is_empty(), "no handler failure files written");

    // Re-run with `runs = 0` so the campaign cannot rediscover the bug; the failure must
    // come from replaying the persisted file. Then prove the stale-file deletion path by
    // pointing the test at a new contract (different selector → no match) and confirming
    // the orphaned file is removed.
    prj.update_config(|config| {
        config.invariant.runs = 0;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok"]).assert_failure().stderr_eq(str![[r#"
...
Warning: Replayed handler-side assertion bug from [..]
...
"#]]);

    // Sanity check: persisted file is still there after a successful replay.
    let entries_after: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(entries_after.len(), entries.len(), "replayed file should be preserved");

    // Replace the asserting handler with a no-op so the persisted sequence no longer
    // reproduces. The replay step must delete the stale file in place.
    prj.add_source(
        "AlwaysAssert.sol",
        r#"
contract AlwaysAssert {
    function boom() external {}
}
   "#,
    );
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok"]).assert_success();
    let entries_stale: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert!(entries_stale.is_empty(), "stale handler failure file should be deleted");

    // A stale handler file co-existing with a primary invariant failure must NOT be
    // mis-identified as the handler bug that's still reproducing. Re-persist a handler bug
    // first, then patch the handler to no-op AND make `invariant_ok` assert. Replay must
    // delete the (now stale) handler file and surface the failure as a primary invariant
    // failure, not under `Assertion Tests:`.
    prj.add_source(
        "AlwaysAssert.sol",
        r#"
contract AlwaysAssert {
    function boom() external { assert(false); }
}
   "#,
    );
    prj.update_config(|config| {
        config.invariant.runs = 1;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok"]).assert_failure();
    assert!(handlers_dir.read_dir().unwrap().next().is_some(), "handler bug should re-persist");

    prj.add_source(
        "AlwaysAssert.sol",
        r#"
contract AlwaysAssert {
    function boom() external {}
}
   "#,
    );
    prj.add_test(
        "AlwaysAssertTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {AlwaysAssert} from "../src/AlwaysAssert.sol";

contract AlwaysAssertTest is Test {
    AlwaysAssert h;
    function setUp() public { h = new AlwaysAssert(); targetContract(address(h)); }
    function invariant_ok() public { assert(false); }
}
   "#,
    );
    prj.update_config(|config| {
        config.invariant.runs = 0;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: panic: assertion failed (0x01)] invariant_ok()[..]
...
"#]]);
    let entries_after: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert!(entries_after.is_empty(), "stale handler file must be deleted, got {entries_after:?}");
});

// Two distinct handler contracts → two persisted files, both reported.
forgetest_init!(multi_handler_bugs_each_persist_independently, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 20;
        config.invariant.fail_on_revert = false;
    });
    prj.add_source(
        "HandlerA.sol",
        r#"
contract HandlerA {
    function boomA() external { assert(false); }
}
   "#,
    );
    prj.add_source(
        "HandlerB.sol",
        r#"
contract HandlerB {
    function boomB() external { assert(false); }
}
   "#,
    );
    prj.add_test(
        "MultiHandlerTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {HandlerA} from "../src/HandlerA.sol";
import {HandlerB} from "../src/HandlerB.sol";

contract MultiHandlerTest is Test {
    HandlerA a;
    HandlerB b;

    function setUp() public {
        a = new HandlerA();
        b = new HandlerB();
        targetContract(address(a));
        targetContract(address(b));
    }

    function invariant_ok() public view {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_ok"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (2018): Function state mutability can be restricted to pure
 [FILE]:5:5:
  |
5 |     function boomA() external { assert(false); }
  |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Warning (2018): Function state mutability can be restricted to pure
 [FILE]:5:5:
  |
5 |     function boomB() external { assert(false); }
  |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^


Ran 1 test for test/MultiHandlerTest.t.sol:MultiHandlerTest
Assertion Tests: 2 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/HandlerB.sol:HandlerB::boomB
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[src/HandlerB.sol:HandlerB]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=boomB() args=[]
[FAIL: panic: assertion failed (0x01)] src/HandlerA.sol:HandlerA::boomA
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[src/HandlerA.sol:HandlerA]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=boomA() args=[]
 invariant_ok() (runs: 1, calls: 20, reverts: 20)

╭----------+----------+-------+---------+----------╮
| Contract | Selector | Calls | Reverts | Discards |
+==================================================+
| HandlerA | boomA    | [..]  | [..]    | 0        |
|----------+----------+-------+---------+----------|
| HandlerB | boomB    | [..]  | [..]    | 0        |
╰----------+----------+-------+---------+----------╯

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/MultiHandlerTest.t.sol:MultiHandlerTest
Assertion Tests: 2 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/HandlerB.sol:HandlerB::boomB
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[src/HandlerB.sol:HandlerB]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=boomB() args=[]
[FAIL: panic: assertion failed (0x01)] src/HandlerA.sol:HandlerA::boomA
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[src/HandlerA.sol:HandlerA]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=boomA() args=[]
 invariant_ok() (runs: 1, calls: 20, reverts: 20)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);

    let handlers_dir = prj
        .root()
        .join("cache")
        .join("invariant")
        .join("failures")
        .join("MultiHandlerTest")
        .join("handlers");
    let entries: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(entries.len(), 2, "expected one persisted file per handler bug");
});

// Persisted file holds the post-shrink sequence; replay renders `(original: M, shrunk: M)`
// instead of regrowing.
forgetest_init!(handler_bug_replay_is_idempotent_after_shrink, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 50;
        config.invariant.fail_on_revert = false;
    });
    prj.add_source(
        "ShrinkableHandler.sol",
        r#"
contract ShrinkableHandler {
    // Filler so the campaign produces a prefix that has to be shrunk away.
    function noop() external {}
    function boom() external { assert(false); }
}
   "#,
    );
    prj.add_test(
        "ShrinkReplayTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {ShrinkableHandler} from "../src/ShrinkableHandler.sol";

contract ShrinkReplayTest is Test {
    ShrinkableHandler h;
    function setUp() public { h = new ShrinkableHandler(); targetContract(address(h)); }
    function invariant_ok() public view {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_ok"]).assert_failure();

    let handlers_dir = prj
        .root()
        .join("cache")
        .join("invariant")
        .join("failures")
        .join("ShrinkReplayTest")
        .join("handlers");
    let file = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .expect("persisted handler file");
    let json: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(file.path()).unwrap()).unwrap();
    let persisted_len = json["call_sequence"].as_array().unwrap().len();
    assert_eq!(persisted_len, 1, "persisted file must hold the shrunk (anchor-only) sequence");

    // Re-run with runs = 0 so the failure must come from replay; assert the rendered
    // (original: N, shrunk: M) shows N == M == 1, proving replay is idempotent.
    prj.update_config(|config| {
        config.invariant.runs = 0;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/ShrinkReplayTest.t.sol:ShrinkReplayTest
Assertion Tests: 1 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/ShrinkableHandler.sol:ShrinkableHandler::boom
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[src/ShrinkableHandler.sol:ShrinkableHandler]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=boom() args=[]
 invariant_ok() (runs: 0, calls: 0, reverts: 0)
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/ShrinkReplayTest.t.sol:ShrinkReplayTest
Assertion Tests: 1 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/ShrinkableHandler.sol:ShrinkableHandler::boom
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[src/ShrinkableHandler.sol:ShrinkableHandler]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=boom() args=[]
 invariant_ok() (runs: 0, calls: 0, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
});

// `InvariantSettings` change between runs → persisted file is skipped with a warning and
// left intact for a future compatible run.
forgetest_init!(handler_persisted_failure_skipped_on_settings_change, |prj, cmd| {
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
        "SettingsChangeTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {AlwaysAssert} from "../src/AlwaysAssert.sol";

contract SettingsChangeTest is Test {
    AlwaysAssert h;
    function setUp() public { h = new AlwaysAssert(); targetContract(address(h)); }
    function invariant_ok() public view {}
}
   "#,
    );

    cmd.args(["test", "--mt", "invariant_ok"]).assert_failure();

    let handlers_dir = prj
        .root()
        .join("cache")
        .join("invariant")
        .join("failures")
        .join("SettingsChangeTest")
        .join("handlers");
    let entries_before: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(entries_before.len(), 1, "expected one persisted handler file");

    // Flip a tracked InvariantSettings field (`fail_on_revert`) so the persisted file is
    // incompatible. Combined with runs = 0, no campaign runs and no replay fires, so the
    // test must pass.
    prj.update_config(|config| {
        config.invariant.fail_on_revert = true;
        config.invariant.runs = 0;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok"]).assert_success().stderr_eq(str![[r#"
...
Warning: Failure from [..] file was ignored because invariant test settings have changed: [..]
...
"#]]);

    // Settings-mismatched files must be left intact (only stale-but-compatible files are
    // deleted), so a future run with the original settings can still pick them up.
    let entries_after: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(entries_after.len(), 1, "settings-incompatible file should be preserved");
});

// Pre-anchor assertion = different bug: replay must delete the file, not keep it.
forgetest_init!(handler_persisted_failure_deleted_when_earlier_call_now_asserts, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 50;
        config.invariant.depth = 20;
        config.invariant.fail_on_revert = false;
    });
    // `boom` only asserts after `setup` was called → persisted sequence must contain both
    // calls (`setup` cannot be shrunk away without losing the assertion).
    prj.add_source(
        "TwoStep.sol",
        r#"
contract TwoStep {
    bool primed;
    function setup() external { primed = true; }
    function boom() external { if (primed) assert(false); }
}
   "#,
    );
    prj.add_test(
        "TwoStepTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {TwoStep} from "../src/TwoStep.sol";

contract TwoStepTest is Test {
    TwoStep h;
    function setUp() public { h = new TwoStep(); targetContract(address(h)); }
    function invariant_ok() public view {}
}
   "#,
    );
    cmd.args(["test", "--mt", "invariant_ok"]).assert_failure();

    let handlers_dir = prj
        .root()
        .join("cache")
        .join("invariant")
        .join("failures")
        .join("TwoStepTest")
        .join("handlers");
    let file = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|x| x == "json"))
        .expect("persisted handler file");
    let json: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(file.path()).unwrap()).unwrap();
    let len = json["call_sequence"].as_array().unwrap().len();
    assert!(len >= 2, "sequence must include both setup() and boom() calls, got {len}");

    // Patch `setup` to assert. Replay now sees a pre-anchor assertion → file is stale.
    prj.add_source(
        "TwoStep.sol",
        r#"
contract TwoStep {
    bool primed;
    function setup() external { assert(false); }
    function boom() external { if (primed) assert(false); }
}
   "#,
    );
    prj.update_config(|config| {
        config.invariant.runs = 0;
    });
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok"]).assert_success();

    let entries_after: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert!(
        entries_after.is_empty(),
        "stale file (earlier call asserts) must be deleted, got {entries_after:?}"
    );
});

// Two branches that both assert in the same handler function collapse to a single
// persisted bug under site-granular dedup, regardless of whether one branch requires
// a setup call. The shrinker minimizes the kept reproducer to whichever branch is
// reachable with the fewest calls (typically the no-setup-required branch).
forgetest_init!(handler_two_branches_same_function_collapse_to_one_bug, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 100;
        config.invariant.depth = 20;
        config.invariant.fail_on_revert = false;
        config.invariant.corpus.corpus_dir = Some("inv_corpus".into());
    });
    prj.add_source(
        "TwoBranch.sol",
        r#"
contract TwoBranch {
    bool routed;
    function route() external { routed = true; }
    function check() external {
        if (routed) {
            assert(false);
        } else {
            assert(false);
        }
    }
}
   "#,
    );
    prj.add_test(
        "TwoBranchTest.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {TwoBranch} from "../src/TwoBranch.sol";

contract TwoBranchTest is Test {
    TwoBranch h;
    function setUp() public { h = new TwoBranch(); targetContract(address(h)); }
    function invariant_ok() public view {}
}
   "#,
    );
    cmd.args(["test", "--mt", "invariant_ok"]).assert_failure();

    let handlers_dir = prj
        .root()
        .join("cache")
        .join("invariant")
        .join("failures")
        .join("TwoBranchTest")
        .join("handlers");
    let entries: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    // Both branches share `(reverter, check_selector)` → 1 persisted bug under
    // site-granular dedup.
    assert_eq!(
        entries.len(),
        1,
        "expected 1 persisted handler bug for two branches in same function, got {entries:?}",
    );
});

// Regression: with `assertions_revert = false`, a non-reverting `vm.assert*` writes
// `GLOBAL_FAIL_SLOT = 1` and that committed slot was never cleared, poisoning every
// subsequent `handlers_succeeded` check and silently suppressing later `assert_invariants`
// evaluations. Asserts on call #1, then bumps a counter on call #2/#3 to trip a real
// predicate — both the handler bug AND the predicate failure must be reported.
forgetest_init!(handler_vm_assert_global_flag_does_not_poison_invariant_checks, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 5;
        config.invariant.fail_on_revert = false;
        config.assertions_revert = false;
    });
    prj.add_test(
        "HandlerVmAssertPoisonTest.t.sol",
        r#"
import "forge-std/Test.sol";

contract OncePoisonHandler is Test {
    uint256 public counter;
    bool public asserted;

    // First call sets GLOBAL_FAIL_SLOT via the non-reverting cheatcode path; later
    // calls don't re-touch the slot, so invariants only keep getting checked if the
    // committed slot is cleared after recording the handler bug.
    function step() external {
        counter++;
        if (!asserted) {
            asserted = true;
            vm.assertEq(uint256(1), uint256(2));
        }
    }
}

contract HandlerVmAssertPoisonTest is Test {
    OncePoisonHandler handler;

    function setUp() public {
        handler = new OncePoisonHandler();
        targetContract(address(handler));
    }

    // Holds at counter ∈ {1, 2}, breaks at counter == 3.
    function invariant_counter_below_three() public view {
        require(handler.counter() < 3, "counter reached 3");
    }
}
"#,
    );

    cmd.args(["test", "--mt", "invariant_counter_below_three"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/HandlerVmAssertPoisonTest.t.sol:HandlerVmAssertPoisonTest
[FAIL: counter reached 3]
	[Sequence] (original: 3, shrunk: 1)
		sender=[..] addr=[test/HandlerVmAssertPoisonTest.t.sol:OncePoisonHandler]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step() args=[]

Assertion Tests: 1 assertion bug(s) found
[FAIL: assertion failed] test/HandlerVmAssertPoisonTest.t.sol:OncePoisonHandler::step
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[test/HandlerVmAssertPoisonTest.t.sol:OncePoisonHandler]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step() args=[]
 invariant_counter_below_three() (runs: 1, calls: 3, reverts: 0)

╭-------------------+----------+-------+---------+----------╮
| Contract          | Selector | Calls | Reverts | Discards |
+===========================================================+
| OncePoisonHandler | step     | 3     | 0       | 0        |
╰-------------------+----------+-------+---------+----------╯

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/HandlerVmAssertPoisonTest.t.sol:HandlerVmAssertPoisonTest
[FAIL: counter reached 3]
	[Sequence] (original: 3, shrunk: 1)
		sender=[..] addr=[test/HandlerVmAssertPoisonTest.t.sol:OncePoisonHandler]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step() args=[]

Assertion Tests: 1 assertion bug(s) found
[FAIL: assertion failed] test/HandlerVmAssertPoisonTest.t.sol:OncePoisonHandler::step
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[test/HandlerVmAssertPoisonTest.t.sol:OncePoisonHandler]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step() args=[]
 invariant_counter_below_three() (runs: 1, calls: 3, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
});
