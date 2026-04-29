//! E2E tests for handler-side assertion bugs: dedup by edge fingerprint, persistence
//! under `failures/<contract>/handlers/<fingerprint>.json`, replay/shrink semantics, and
//! stale-file cleanup. Distinct from invariant predicate failures.

use foundry_test_utils::{forgetest_init, str};

// Handler `assert(false)` surfaces under `Suite handlers:`, not as a live invariant failure.
forgetest_init!(assert_all_handler_assertion_routed_to_handler_section, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 10;
        config.invariant.fail_on_revert = false;
        config.invariant.assert_all = true;
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

    cmd.args(["test", "--mt", "invariant_a"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/AssertAllAssertTest.t.sol:AssertAllAssertTest
...
Suite assert_all: 0/2 invariants broken

Suite handlers: 1 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/AssertHandler.sol:AssertHandler::alwaysAssert
	[Sequence] (original: [..], shrunk: [..])
...
"#]]);
});

// Edge-coverage dedup: 4 distinct paths through one selector → 4 separate bugs, each
// shrunk to its anchor. Then collapsing all 4 branches into one path triggers fingerprint
// mismatch on replay → all stale files deleted.
forgetest_init!(handler_assertion_dedupes_by_edge_coverage, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 1;
        config.invariant.depth = 200;
        config.invariant.fail_on_revert = false;
        config.invariant.assert_all = true;
        config.invariant.corpus.corpus_dir = Some("inv_corpus".into());
    });
    prj.add_source(
        "MultiPathHandler.sol",
        r#"
contract MultiPathHandler {
    uint256 public state;

    // Filler so fuzzing produces a prefix to shrink.
    function noop() external {}

    // Four branches, one selector — distinct edge fingerprints per path.
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

    // Seed pinned for stable shrink counts; concrete originals (2, 22, 11, 7) confirm
    // shrinking actually fired. Update the snapshot if upstream RNG shifts the numbers.
    cmd.args(["test", "--mt", "invariant_ok", "--fuzz-seed", "119"]).assert_failure().stdout_eq(
        str![[r#"
...
Suite handlers: 4 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/MultiPathHandler.sol:MultiPathHandler::maybeAssert
	[Sequence] (original: 2, shrunk: 1)
		sender=[..] addr=[..] calldata=maybeAssert(uint8) args=[2]
[FAIL: panic: assertion failed (0x01)] src/MultiPathHandler.sol:MultiPathHandler::maybeAssert
	[Sequence] (original: 22, shrunk: 1)
		sender=[..] addr=[..] calldata=maybeAssert(uint8) args=[66]
[FAIL: panic: assertion failed (0x01)] src/MultiPathHandler.sol:MultiPathHandler::maybeAssert
	[Sequence] (original: 11, shrunk: 1)
		sender=[..] addr=[..] calldata=maybeAssert(uint8) args=[148]
[FAIL: panic: assertion failed (0x01)] src/MultiPathHandler.sol:MultiPathHandler::maybeAssert
	[Sequence] (original: 7, shrunk: 1)
		sender=[..] addr=[..] calldata=maybeAssert(uint8) args=[253]
...
"#]],
    );

    // Stale-fingerprint deletion: collapse all 4 branches into a single uniform path so the
    // same calldata still asserts but takes a different code path → recomputed fingerprint
    // no longer matches any of the persisted filenames. Replay must delete every stale file.
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
    assert_eq!(before.len(), 4, "expected 4 persisted handler bugs before patch");

    prj.add_source(
        "MultiPathHandler.sol",
        r#"
contract MultiPathHandler {
    uint256 public state;
    function noop() external {}
    // Single uniform branch: every input now hits the same edge fingerprint.
    function maybeAssert(uint8 path) external {
        state = uint256(path) % 4 + 1;
        assert(false);
    }
}
   "#,
    );
    prj.update_config(|config| {
        config.invariant.runs = 0;
    });
    // All 4 persisted fingerprints are stale (the patched contract has a single uniform
    // path with a different fingerprint). With `runs=0` there's no fuzzing to rediscover
    // the bug under a fresh fingerprint, so no handler bug should survive replay → success.
    cmd.forge_fuse().args(["test", "--mt", "invariant_ok", "--fuzz-seed", "119"]).assert_success();

    let after: Vec<_> = std::fs::read_dir(&handlers_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert!(after.is_empty(), "all stale fingerprint files should be deleted, got {after:?}");
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
    // failure, not under `Suite handlers:`.
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
        config.invariant.assert_all = true;
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
...
Suite handlers: 2 assertion bug(s) found
...
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
...
Suite handlers: 1 assertion bug(s) found
[FAIL: panic: assertion failed (0x01)] src/ShrinkableHandler.sol:ShrinkableHandler::boom
	[Sequence] (original: 1, shrunk: 1)
		sender=[..] addr=[..] calldata=boom() args=[]
...
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

// Shrink rejects candidates whose anchor takes a different edge path: a setup call that
// routes the anchor through branch A is kept, even though dropping it would still assert.
forgetest_init!(handler_shrink_keeps_setup_call_when_required_for_same_path, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.runs = 100;
        config.invariant.depth = 20;
        config.invariant.fail_on_revert = false;
        config.invariant.assert_all = true;
        // Edge-coverage dedup requires the corpus to be enabled; otherwise
        // `handler_edge_fingerprint` falls back to `(reverter, selector)` and the two
        // branches collapse to a single fingerprint.
        config.invariant.corpus.corpus_dir = Some("inv_corpus".into());
    });
    // Two branches that both assert; `route()` flips state so `check()` takes branch A.
    prj.add_source(
        "TwoBranch.sol",
        r#"
contract TwoBranch {
    bool routed;
    function route() external { routed = true; }
    function check() external {
        if (routed) {
            // Branch A — distinct edge fingerprint vs branch B.
            assert(false);
        } else {
            // Branch B.
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
    // Each branch is a distinct fingerprint → 2 persisted bugs.
    assert_eq!(entries.len(), 2, "expected 2 persisted handler bugs (one per branch)");

    // The branch-A bug requires `route()` before `check()`. Without the fingerprint check
    // the shrinker would drop `route()` and silently keep the file under the (now stale)
    // branch-A fingerprint. With the fix, that file's call_sequence stays length 2.
    let max_len = entries
        .iter()
        .map(|e| {
            let v: serde_json::Value =
                serde_json::from_reader(std::fs::File::open(e.path()).unwrap()).unwrap();
            v["call_sequence"].as_array().unwrap().len()
        })
        .max()
        .unwrap();
    assert!(
        max_len >= 2,
        "branch-A bug must keep its `route()` setup call after shrink, max_len={max_len}"
    );
});
