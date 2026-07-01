//! Tests for AFL-`afl-showmap`-style corpus replay (`forge test --showmap-out`).

use foundry_test_utils::str;

// Locate a per-test approach dir by suffix (suite/test ids include project-dependent paths).
fn find_approach_dir(out: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    std::fs::read_dir(out)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", out.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            p.is_dir()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(suffix))
        })
        .unwrap_or_else(|| panic!("no dir ending with {suffix} in {}", out.display()))
}

// Generate a corpus by running an invariant + fuzz test, then replay it via
// `--showmap-out` and verify that showmap files are produced under the
// expected `<approach>__<suite>__<test>/<trial>.txt` layout with hex-prefixed IDs.
forgetest_init!(showmap_replay_emits_files, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 5;
        config.invariant.corpus.corpus_dir = Some("invariant_corpus".into());

        config.fuzz.runs = 5;
        config.fuzz.corpus.corpus_dir = Some("fuzz_corpus".into());
    });
    prj.add_test(
        "ShowmapCounter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract ShowmapCounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
        counter.setNumber(0);
    }

    function testFuzz_SetNumber(uint256 x) public {
        counter.setNumber(x);
        assertEq(counter.number(), x);
    }

    function invariant_counter_called() public view {}
}
   "#,
    );

    // Phase 1: build a corpus.
    cmd.args(["test", "--mc", "ShowmapCounterTest"]).assert_success();

    // Phase 2: replay it through showmap with an explicit trial id.
    cmd.forge_fuse()
        .args([
            "test",
            "--mc",
            "ShowmapCounterTest",
            "--showmap-out",
            "showmap_out",
            "--showmap-approach",
            "replay",
            "--showmap-trial",
            "t1",
        ])
        .assert_success();

    // Verify files were produced. Both fuzz and invariant tests get a per-(anchor)
    // function approach dir so contracts with multiple campaigns don't collide.
    let out = prj.root().join("showmap_out");
    let invariant_dir = find_approach_dir(&out, "ShowmapCounterTest__invariant_counter_called");
    let fuzz_dir = find_approach_dir(&out, "ShowmapCounterTest__testFuzz_SetNumber");
    let invariant_file = invariant_dir.join("t1.txt");
    let fuzz_file = fuzz_dir.join("t1.txt");
    assert!(invariant_file.exists(), "missing {}", invariant_file.display());
    assert!(fuzz_file.exists(), "missing {}", fuzz_file.display());

    // Sanity-check format: every line is `evm_<hash16>_<pc>:<count>` with count > 0.
    for f in [&invariant_file, &fuzz_file] {
        let body = std::fs::read_to_string(f).unwrap();
        assert!(!body.is_empty(), "{} is empty", f.display());
        for line in body.lines() {
            let (id, count) = line.split_once(':').expect("missing colon");
            assert!(id.starts_with("evm_"), "unexpected id in {}: {line}", f.display());
            // expect three underscore-separated parts: prefix, hash16, pc
            let parts: Vec<_> = id.splitn(3, '_').collect();
            assert_eq!(parts.len(), 3, "malformed id in {}: {line}", f.display());
            assert_eq!(
                parts[1].len(),
                16,
                "hash prefix should be 16 hex chars in {}: {line}",
                f.display()
            );
            let n: u64 = count.parse().expect("count parses");
            assert!(n > 0, "zero count in {}: {line}", f.display());
        }
    }

    // Showmap mode reports replay results, not regular test results.
    cmd.forge_fuse()
        .args(["test", "--mc", "ShowmapCounterTest", "--showmap-out", "showmap_out2"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 2 tests for test/ShowmapCounter.t.sol:ShowmapCounterTest
[PASS] invariant_counter_called() (replay: [..] entries, [..] files)
[PASS] testFuzz_SetNumber(uint256) (replay: [..] entries, [..] files)
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});

// Per-input mode emits one file per corpus entry.
forgetest_init!(showmap_replay_per_input_emits_one_file_per_entry, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 5;
        config.invariant.corpus.corpus_dir = Some("invariant_corpus".into());
    });
    prj.add_test(
        "ShowmapCounter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract ShowmapCounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_counter_called() public view {}
}
   "#,
    );

    cmd.args(["test", "--mc", "ShowmapCounterTest"]).assert_success();

    cmd.forge_fuse()
        .args([
            "test",
            "--mc",
            "ShowmapCounterTest",
            "--showmap-out",
            "showmap_out",
            "--showmap-per-input",
        ])
        .assert_success();

    // Per-input mode writes one file per corpus entry inside the test's approach dir.
    let out = prj.root().join("showmap_out");
    let approach_dir = find_approach_dir(&out, "ShowmapCounterTest__invariant_counter_called");
    let entries: Vec<_> = std::fs::read_dir(&approach_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("txt"))
        .collect();
    assert!(!entries.is_empty(), "expected per-entry files in {}", approach_dir.display());
});

// Reruns with distinct `--showmap-trial` values must accumulate side-by-side
// instead of overwriting each other.
forgetest_init!(showmap_replay_distinct_trials_accumulate, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.invariant.runs = 5;
        config.invariant.depth = 5;
        config.invariant.corpus.corpus_dir = Some("invariant_corpus".into());
    });
    prj.add_test(
        "ShowmapCounter.t.sol",
        r#"
import {Test} from "forge-std/Test.sol";
import {Counter} from "../src/Counter.sol";

contract ShowmapCounterTest is Test {
    Counter public counter;

    function setUp() public {
        counter = new Counter();
    }

    function invariant_counter_called() public view {}
}
   "#,
    );

    // Build a corpus first.
    cmd.args(["test", "--mc", "ShowmapCounterTest"]).assert_success();

    // Two reruns with distinct trial ids must produce two distinct files.
    for trial in ["t1", "t2"] {
        cmd.forge_fuse()
            .args([
                "test",
                "--mc",
                "ShowmapCounterTest",
                "--showmap-out",
                "showmap_out",
                "--showmap-trial",
                trial,
            ])
            .assert_success();
    }

    // Distinct trials become side-by-side files inside the same per-test approach dir.
    let out = prj.root().join("showmap_out");
    let approach_dir = find_approach_dir(&out, "ShowmapCounterTest__invariant_counter_called");
    let t1 = approach_dir.join("t1.txt");
    let t2 = approach_dir.join("t2.txt");
    assert!(t1.exists(), "missing trial 1 file {}", t1.display());
    assert!(t2.exists(), "missing trial 2 file {}", t2.display());

    let before = std::fs::read_to_string(&t1).unwrap();
    let retry = cmd
        .forge_fuse()
        .args([
            "test",
            "--mc",
            "ShowmapCounterTest",
            "--showmap-out",
            "showmap_out",
            "--showmap-trial",
            "t1",
        ])
        .assert_failure();
    let stdout = String::from_utf8(retry.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("pick a different --showmap-trial"), "{stdout}");
    assert_eq!(std::fs::read_to_string(&t1).unwrap(), before);
});

forgetest_init!(showmap_replay_rejects_path_component_names, |prj, cmd| {
    prj.add_test(
        "ShowmapCounter.t.sol",
        r#"
contract ShowmapCounterTest {
    function invariant_counter_called() public view {}
}
   "#,
    );

    for args in [
        vec![
            "test",
            "--mc",
            "ShowmapCounterTest",
            "--showmap-out",
            "showmap_out",
            "--showmap-approach",
            "../outside",
        ],
        vec![
            "test",
            "--mc",
            "ShowmapCounterTest",
            "--showmap-out",
            "showmap_out",
            "--showmap-trial",
            "../../victim",
        ],
    ] {
        let result = cmd.forge_fuse().args(args).assert_failure();
        let stderr = String::from_utf8(result.get_output().stderr.clone()).unwrap();
        assert!(stderr.contains("expected a single file-name component"), "{stderr}");
    }
});

forgetest_init!(showmap_replay_rejects_empty_corpus_dir, |prj, cmd| {
    prj.add_test(
        "ShowmapCounter.t.sol",
        r#"
contract ShowmapCounterTest {
    function testFuzz_value(uint256 value) public pure {
        value;
    }
}
   "#,
    );
    std::fs::create_dir_all(prj.root().join("empty_corpus")).unwrap();

    let result = cmd
        .args([
            "test",
            "--mc",
            "ShowmapCounterTest",
            "--showmap-out",
            "showmap_out",
            "--showmap-corpus-dir",
            "empty_corpus",
        ])
        .assert_failure();
    let stdout = String::from_utf8(result.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("corpus directory not found: empty_corpus"), "{stdout}");
});
