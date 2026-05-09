//! Tests for AFL-`afl-showmap`-style corpus replay (`forge test --showmap-out`).

// Generate a corpus by running an invariant + fuzz test, then replay it via
// `--showmap-out` and verify that showmap files are produced under the
// expected `<approach>/<contract>__<test>.txt` layout with hex-prefixed IDs.
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

    // Phase 2: replay it through showmap.
    cmd.forge_fuse()
        .args([
            "test",
            "--mc",
            "ShowmapCounterTest",
            "--showmap-out",
            "showmap_out",
            "--showmap-approach",
            "replay",
        ])
        .assert_success();

    // Verify files were produced under <out>/<approach>/<contract>__<test>.txt.
    let approach_dir = prj.root().join("showmap_out").join("replay");
    let invariant_file = approach_dir.join("ShowmapCounterTest__invariant_counter_called.txt");
    let fuzz_file = approach_dir.join("ShowmapCounterTest__testFuzz_SetNumber.txt");
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
    let assert = cmd
        .forge_fuse()
        .args(["test", "--mc", "ShowmapCounterTest", "--showmap-out", "showmap_out2"])
        .assert_success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    // Order-insensitive: both lines must appear, replay summary format must match.
    assert!(
        stdout.contains("invariant_counter_called() (replay:")
            && stdout.contains("entries,")
            && stdout.contains("files)"),
        "missing invariant replay line in:\n{stdout}"
    );
    assert!(
        stdout.contains("testFuzz_SetNumber(uint256) (replay:"),
        "missing fuzz replay line in:\n{stdout}"
    );
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

    let approach_dir = prj.root().join("showmap_out").join("replay");
    let entries: Vec<_> = std::fs::read_dir(&approach_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("ShowmapCounterTest__invariant_counter_called__"))
                .unwrap_or(false)
        })
        .collect();
    assert!(!entries.is_empty(), "expected per-entry files in {}", approach_dir.display());
});
