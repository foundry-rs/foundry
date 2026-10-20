//! Tests for the `forge properties` command. Generators are shell scripts, so the tests run only
//! on Unix.
#![cfg(unix)]

use foundry_compilers::artifacts::remappings::Remapping;
use foundry_test_utils::{TestProject, assert_data_eq, forgetest_init, str};
use std::{fs, os::unix::fs::PermissionsExt, str::FromStr};

/// Starts a generator script. `check FILE` runs the prompt's check command on a candidate file.
const PRELUDE: &str = r#"#!/bin/sh
set -eu
prompt="$1"
output="$2"
check() {
    set -- $(sed -n '/"check_command": \[/,/\]/p' "$prompt" | sed -e '1d;$d' -e 's/^ *"//' -e 's/",*$//' -e "s|^CANDIDATE_JSON\$|$1|")
    "$@" > /dev/null 2>&1 || true
}
"#;

const FEE: &str = r#"
pragma solidity ^0.8.20;

/// @notice Charges 1% of `amount`, capped at 100 ether.
library Fee {
    function fee(uint256 amount) internal pure returns (uint256 f) {
        f = amount / 100;
        if (f > 10 ether) f = 10 ether;
    }

    function charges(uint256 amount) internal pure returns (bool) {
        return amount / 100 > 0;
    }
}
"#;

/// Writes the brief and the generator script and returns the common arguments.
fn properties_args(prj: &TestProject, script: &str) -> Vec<String> {
    let brief = prj.root().join("brief.md");
    fs::write(&brief, "Find bugs in the targets and prove each one with a test.").unwrap();
    let generator = prj.root().join("generator.sh");
    fs::write(&generator, format!("{PRELUDE}{script}")).unwrap();
    fs::set_permissions(&generator, fs::Permissions::from_mode(0o755)).unwrap();
    [
        "properties",
        "--root",
        prj.root().to_str().unwrap(),
        "--brief",
        brief.to_str().unwrap(),
        "--generator",
        generator.to_str().unwrap(),
        "--seed",
        "0x5eed",
        "--seed",
        "0xc0ffee",
    ]
    .map(String::from)
    .to_vec()
}

/// Returns the first agent's `rounds.json`.
fn rounds(prj: &TestProject) -> serde_json::Value {
    let rounds =
        fs::read_to_string(prj.root().join("cache/properties/agent-1/rounds.json")).unwrap();
    serde_json::from_str(&rounds).unwrap()
}

// Three agents run in parallel. Forge reports a possible bug only when two agents agree on its
// root cause, keeps the other failures in the report file, keeps one copy of identical passing
// files, and sums the usage that agents report.
#[forgetest_init]
fn properties_reports_bugs_that_agents_agree_on(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    fs::write(prj.root().join("README.md"), "readme-marker").unwrap();
    let context = prj.root().join("context.md");
    fs::write(&context, "context-marker: rounding notes").unwrap();
    let mut args = properties_args(
        &prj,
        r#"
agent=$(sed -n 's/^    "id": \([0-9][0-9]*\),$/\1/p' "$prompt" | head -n 1)
case "$agent" in 1) focus=spec ;; 2) focus=state ;; 3) focus=code ;; esac
grep -q "\"lens\": \"$focus: " "$prompt"
grep -q '"count": 3' "$prompt"
grep -q "\"allowed_path_prefix\": \"test/generated/agent-$agent\"" "$prompt"
grep -q '"path": "src/Fee.sol"' "$prompt"
grep -q 'context-marker: rounding notes' "$prompt"
# Each agent works in its own copy of the project, with the project's other files.
grep -q readme-marker README.md
dir="test/generated/agent-$agent"
spec='{"path": "'$dir'/FeeSpec.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeSpecTest {\n    function testSmallFee() public pure {\n        require(Fee.fee(100) == 1);\n    }\n}\n"}'
usage='"usage": {"input_tokens": 1000, "output_tokens": 200, "cost_usd": 0.01}'
case "$agent" in
1) cat > "$output" <<JSON
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "cap: finding",
  "files": [$spec, {"path": "$dir/FeeCap.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeCapTest {\n    function testDocumentedCap() public pure {\n        require(Fee.fee(1e30) == 100 ether);\n    }\n}\n"}],
  "hypotheses": [{"id": "testDocumentedCap", "lens": "spec", "function": "Fee.fee", "claim": "capped at 100 ether", "recipe": "fee(1e30)", "result": "finding"}],
  "evidence": {"FeeCapTest::testDocumentedCap": {"claim": "The fee is capped at 100 ether.", "quote": "Charges 1% of \`amount\`, capped at 100 ether.", "source": "src/Fee.sol:3", "basis": "documented", "code_path": "src/Fee.sol:7 fee() caps the fee at 10 ether."}},
  $usage
}
JSON
;;
2) cat > "$output" <<JSON
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "cap: finding; rounding: refuted",
  "files": [$spec, {"path": "$dir/FeeRounding.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeRoundingTest {\n    function testRoundsDown() public pure {\n        require(Fee.fee(199) == 1);\n    }\n}\n"}, {"path": "$dir/FeeLimit.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeLimitTest {\n    function testReachesCap() public pure {\n        require(Fee.fee(2e22) == 100 ether);\n    }\n}\n"}],
  "evidence": {"FeeLimitTest::testReachesCap": {"claim": "fee(amount) reaches the documented cap for large amounts.", "quote": "capped at 100 ether", "source": "src/Fee.sol:3", "basis": "documented", "code_path": "src/Fee.sol:7 caps at 10 ether."}},
  $usage
}
JSON
;;
3) cat > "$output" <<JSON
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "charges: finding",
  "files": [$spec, {"path": "$dir/FeeCharges.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeChargesTest {\n    function testChargesZero() public pure {\n        require(Fee.charges(0));\n    }\n}\n"}],
  "evidence": {"FeeChargesTest::testChargesZero": {"claim": "A zero amount is charged.", "quote": "", "source": "the name of charges", "basis": "inferred", "code_path": "src/Fee.sol:11 charges() returns false below 100."}},
  $usage
}
JSON
;;
esac
"#,
    );
    args.extend(["--context", context.to_str().unwrap(), "--agents", "3"].map(String::from));
    cmd.args(args).assert_success().stdout_eq(str![[r#"
possible bug (documented, 2/3 agents): The fee is capped at 100 ether.
  agent 1: FeeCapTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: The fee is capped at 100 ether.
    quote: "Charges 1% of `amount`, capped at 100 ether." (src/Fee.sol:3)
    code path: src/Fee.sol:7 fee() caps the fee at 10 ether.
    fails on every seed; candidate: cache/properties/agent-1/0x[..]
  agent 2: FeeLimitTest::testReachesCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: fee(amount) reaches the documented cap for large amounts.
    quote: "capped at 100 ether" (src/Fee.sol:3)
    code path: src/Fee.sol:7 caps at 10 ether.
    fails on every seed; candidate: cache/properties/agent-2/0x[..]
1 possible bug(s) below the quorum of 2: see cache/properties/report.json
audit agent 1 round 1: audited 1 hypotheses: 1 findings, 0 refuted, 0 not testable
generator usage: 3000 input tokens, 600 output tokens, 0.03 USD
added 2 generated test file(s):
  test/generated/agent-1/FeeSpec.t.sol
  test/generated/agent-2/FeeRounding.t.sol
skipped 2 generated test file(s) that repeat a file of another agent

"#]]);

    let report = fs::read_to_string(prj.root().join("cache/properties/report.json")).unwrap();
    let report = serde_json::from_str::<serde_json::Value>(&report).unwrap();
    let below = &report["below_quorum"][0];
    assert_data_eq!(
        format!("{} {}", below["agents"], below["failures"][0]["test"]),
        str![[r#"[3] "FeeChargesTest::testChargesZero""#]]
    );
    assert_data_eq!(
        report["duplicates"].to_string(),
        str![[
            r#"["test/generated/agent-2/FeeSpec.t.sol","test/generated/agent-3/FeeSpec.t.sol"]"#
        ]]
    );
}

// A kept property must keep its imported helper, while an unrelated proposal file can be dropped.
#[forgetest_init]
fn properties_keeps_imported_candidate_helpers(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let args = properties_args(
        &prj,
        r#"
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "the fee is one percent",
  "files": [
    {"path": "test/generated/agent-1/Helper.sol", "content": "pragma solidity ^0.8.20;\nlibrary Helper { function fee(uint256 amount) internal pure returns (uint256) { return amount / 100; } }\n"},
    {"path": "test/generated/agent-1/Property.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\nimport {Helper} from \"./Helper.sol\";\ncontract PropertyTest { function testFee() public pure { require(Fee.fee(199) == Helper.fee(199)); } }\n"},
    {"path": "test/generated/agent-1/Unused.sol", "content": "pragma solidity ^0.8.20;\nlibrary Unused {}\n"}
  ]
}
JSON
"#,
    );
    cmd.args(args).args(["--agents", "1", "--target", "src/Fee.sol"]).assert_success().stdout_eq(
        str![[r#"
added 2 generated test file(s):
  test/generated/agent-1/Helper.sol
  test/generated/agent-1/Property.t.sol

"#]],
    );

    assert!(!prj.root().join("test/generated/agent-1/Unused.sol").exists());
    cmd.forge_fuse().args(["test", "--match-contract", "PropertyTest"]).assert_success();
}

// A failing test that a generator check saw and the generator then removed is still reported.
#[forgetest_init]
fn properties_reports_removed_check_failures(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let args = properties_args(
        &prj,
        r#"
if grep -q '^  "round": 2,' "$prompt"; then
    echo "generator quota exceeded" >&2
    exit 3
fi
cat > removed.json <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "the documented cap is 100 ether",
  "files": [{"path": "test/generated/agent-1/FeeRemoved.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeRemovedTest {\n    function testDocumentedCap() public pure {\n        require(Fee.fee(1e30) == 100 ether);\n    }\n}\n"}]
}
JSON
check removed.json
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "small fees are 1%",
  "files": [{"path": "test/generated/agent-1/FeeSpec.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeSpecTest {\n    function testSmallFee() public pure {\n        require(Fee.fee(100) == 1);\n    }\n}\n"}]
}
JSON
"#,
    );
    cmd.args(args)
        .args(["--agents", "1", "--target", "src/Fee.sol", "--rounds", "2"])
        .assert_failure()
        .stdout_eq(str![[r#"
possible bug (no evidence, 1/1 agents): FeeRemovedTest::testDocumentedCap
  agent 1: FeeRemovedTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    seen in a generator check, then removed; journal: cache/properties/agent-1/check-failures.jsonl
added 1 generated test file(s):
  test/generated/agent-1/FeeSpec.t.sol

"#]])
        .stderr_eq(str![[r#"
Error: agent 1: generator failed in round 2 (exit status: 3): generator quota exceeded

"#]]);

    // The journal keeps the removed test's source for review.
    let journal =
        fs::read_to_string(prj.root().join("cache/properties/agent-1/check-failures.jsonl"))
            .unwrap();
    let journal = serde_json::from_str::<serde_json::Value>(&journal).unwrap();
    assert!(journal["source"].as_str().unwrap().contains("function testDocumentedCap()"));
}

// The target is in a linked dependency directory. It imports a library through a remapping and the
// test imports the library by its project path; both must name the same file in the workspace, or
// the types differ.
#[forgetest_init]
fn properties_keeps_library_paths_in_workspaces(prj: _, cmd: _) {
    fs::create_dir_all(prj.root().join("lib/example")).unwrap();
    fs::write(
        prj.root().join("lib/example/Kind.sol"),
        "pragma solidity ^0.8.20;\nenum Kind { Small, Large }\ninterface IBucket {\n    /// @notice bucket-spec-marker\n    function bucket(uint256 value) external pure returns (uint256);\n}\n",
    )
    .unwrap();
    fs::create_dir_all(prj.root().join("node_modules/bucket")).unwrap();
    fs::write(
        prj.root().join("node_modules/bucket/Arithmetic.sol"),
        r#"
pragma solidity ^0.8.20;

import {IBucket, Kind} from "example/Kind.sol";

contract Arithmetic is IBucket {
    function bucket(uint256 value) external pure returns (uint256) {
        if (value < 10) return 1;
        if (value < 100) return 2;
        return 3;
    }

    function kind(uint256 value) external pure returns (Kind) {
        return value < 10 ? Kind.Small : Kind.Large;
    }
}
"#,
    )
    .unwrap();
    prj.add_test(
        "Arithmetic.t.sol",
        r#"
pragma solidity ^0.8.20;

import {Kind} from "lib/example/Kind.sol";
import {Arithmetic} from "bucket/Arithmetic.sol";

contract ArithmeticTest {
    Arithmetic internal arithmetic = new Arithmetic();

    function testSmallValue() public view {
        require(arithmetic.bucket(1) == 1);
        require(arithmetic.kind(1) == Kind.Small);
    }
}
"#,
    );
    prj.update_config(|config| {
        config.remappings = ["bucket/=node_modules/bucket/", "example/=lib/example/"]
            .map(|remapping| Remapping::from_str(remapping).unwrap().into())
            .to_vec();
    });
    let args = properties_args(
        &prj,
        r#"
# The prompt has the interface that the target implements.
grep -q 'bucket-spec-marker' "$prompt"
name=Lower
body='require(arithmetic.bucket(9) == 1); require(arithmetic.bucket(10) == 2);'
cat > "$output" <<JSON
{"schema": "foundry/properties-candidate-v1", "rationale": "$name", "files": [{"path": "test/generated/agent-1/Arithmetic$name.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Arithmetic} from \"bucket/Arithmetic.sol\";\ncontract Arithmetic${name}Test {\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testBoundary() public view {\n        $body\n    }\n}\n"}]}
JSON
"#,
    );
    cmd.args(args)
        .args(["--agents", "1", "--target", "node_modules/bucket/Arithmetic.sol"])
        .assert_success()
        .stdout_eq(str![[r#"
added 1 generated test file(s):
  test/generated/agent-1/ArithmeticLower.t.sol

"#]]);
}

// A generated file must not leave the project through a symlinked directory.
#[forgetest_init]
fn properties_rejects_symlinked_generated_directory(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(prj.root().join("test/generated")).unwrap();
    std::os::unix::fs::symlink(outside.path(), prj.root().join("test/generated/agent-1")).unwrap();
    // Internal runs keep their own `--rerun` state; the project's file stays as it is.
    let failures = prj.root().join("cache/test-failures");
    fs::create_dir_all(failures.parent().unwrap()).unwrap();
    fs::write(&failures, r#"{"version":1,"failures":[]}"#).unwrap();
    let args = properties_args(
        &prj,
        r#"
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "small fees are 1%",
  "files": [{"path": "test/generated/agent-1/FeeSpec.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeSpecTest {\n    function testSmallFee() public pure {\n        require(Fee.fee(100) == 1);\n    }\n}\n"}]
}
JSON
"#,
    );
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
no generated test file was kept

"#]]);

    assert_data_eq!(
        rounds(&prj)[0]["reasons"][0].as_str().unwrap(),
        str!["generated test path test/generated/agent-1/FeeSpec.t.sol escapes project root [..]"]
    );
    assert!(!outside.path().join("FeeSpec.t.sol").exists());
    assert_eq!(fs::read_to_string(&failures).unwrap(), r#"{"version":1,"failures":[]}"#);
}

// Checks run in a copy of the project. Inherited config, profiles, `FOUNDRY_CONFIG`, a source
// directory other than `src`, and negative filters must apply as in the project, and filters must
// not hide generated tests.
#[forgetest_init]
fn properties_check_reports_candidate_result(prj: _, cmd: _) {
    fs::write(
        prj.root().join("base.toml"),
        "[profile.default]\nremappings = [\"fee/=contracts/\"]\n",
    )
    .unwrap();
    prj.update_config(|config| {
        config.src = "contracts".into();
        config.test = prj.root().join("test");
        config.test_pattern_inverse = Some(regex::Regex::new(r"^testHidden\w*").unwrap().into());
    });
    let foundry_toml = prj.root().join("foundry.toml");
    let toml = fs::read_to_string(&foundry_toml).unwrap();
    fs::write(
        &foundry_toml,
        toml.replacen("[profile.default]\n", "[profile.default]\nextends = \"base.toml\"\n", 1)
            + "\n[profile.ci]\n",
    )
    .unwrap();
    fs::create_dir_all(prj.root().join("contracts")).unwrap();
    fs::write(
        prj.root().join("contracts/Fee.sol"),
        "pragma solidity ^0.8.20;\nlibrary Fee {\n    function fee(uint256 amount) internal pure returns (uint256) {\n        return amount / 100;\n    }\n}\n",
    )
    .unwrap();
    let write_candidate = |name: &str, content: &str| {
        let candidate = serde_json::json!({
            "schema": "foundry/properties-candidate-v1",
            "rationale": "fee is 1% rounded down",
            "files": [{"path": "test/generated/FeeCheck.t.sol", "content": content}]
        });
        fs::write(prj.root().join(name), candidate.to_string()).unwrap();
    };
    write_candidate(
        "passing.json",
        "pragma solidity ^0.8.20;\nimport {Fee} from \"fee/Fee.sol\";\ncontract FeeCheckTest {\n    /// forge-config: ci.fuzz.runs = 10\n    function testFee() public pure {\n        require(Fee.fee(199) == 1);\n    }\n}\n",
    );
    write_candidate(
        "broken.json",
        "pragma solidity ^0.8.20;\ncontract FeeCheckTest {\n    function testFee() public {\n        missing();\n    }\n}\n",
    );
    // Forge runs every test in the files.
    write_candidate(
        "extra.json",
        "pragma solidity ^0.8.20;\nimport {Fee} from \"../../contracts/Fee.sol\";\ncontract FeeCheckTest {\n    function testFee() public pure {\n        require(Fee.fee(199) == 1);\n    }\n}\ncontract FeeRoundingTest {\n    function testHiddenRoundsUp() public pure {\n        require(Fee.fee(199) == 2);\n    }\n}\n",
    );
    // A merged invariant campaign reports each predicate on its own.
    write_candidate(
        "invariants.json",
        "pragma solidity ^0.8.20;\nimport {Test} from \"forge-std/Test.sol\";\nimport {Fee} from \"../../contracts/Fee.sol\";\ncontract FeeHandler {\n    uint256 public total;\n    function add(uint256 amount) public {\n        total += Fee.fee(amount % 1e30);\n    }\n}\n/// forge-config: default.invariant.runs = 8\n/// forge-config: default.invariant.depth = 8\ncontract FeeInvariantTest is Test {\n    FeeHandler internal handler;\n    function setUp() public {\n        handler = new FeeHandler();\n        targetContract(address(handler));\n    }\n    function invariant_totalFitsSupply() public view {\n        assertLe(handler.total(), 1e30);\n    }\n    function invariant_totalStaysZero() public view {\n        assertEq(handler.total(), 0);\n    }\n}\n",
    );
    // A handler assertion fails the campaign even when every predicate holds.
    write_candidate(
        "handler.json",
        "pragma solidity ^0.8.20;\nimport {Test} from \"forge-std/Test.sol\";\ncontract BrokenHandler {\n    uint256 public pokes;\n    function poke() public {\n        pokes += 1;\n        assert(false);\n    }\n}\n/// forge-config: default.invariant.runs = 8\n/// forge-config: default.invariant.depth = 8\ncontract HandlerInvariantTest is Test {\n    function setUp() public {\n        targetContract(address(new BrokenHandler()));\n    }\n    function invariant_one() public pure {}\n    function invariant_two() public pure {}\n}\n",
    );
    write_candidate(
        "seeded.json",
        "pragma solidity ^0.8.20;\ncontract FeeCheckTest {\n    /// forge-config: default.fuzz.seed = \"0x3\"\n    function testFuzzFee(uint256 amount) public pure {}\n}\n",
    );
    let check = ["properties", "--check", "CANDIDATE", "--seed", "1", "--seed", "2"];
    let check = |candidate| check.map(|arg| if arg == "CANDIDATE" { candidate } else { arg });

    cmd.args(check("passing.json")).assert_success().stdout_eq(str![[r#"
{
  "passed": true,
  "reasons": [],
  "possible_bugs": []
}

"#]]);
    // The resolved workspace test path must win over an absolute inherited environment path.
    cmd.forge_fuse();
    cmd.env("FOUNDRY_TEST", prj.root().join("test"));
    cmd.args(check("passing.json")).assert_success();
    // A config file chosen with `FOUNDRY_CONFIG` is resolved in the project, not in the copy.
    fs::copy(&foundry_toml, prj.root().join("custom.toml")).unwrap();
    cmd.forge_fuse().env("FOUNDRY_CONFIG", "custom.toml");
    cmd.args(check("passing.json")).assert_success();
    cmd.forge_fuse().args(check("broken.json")).assert_failure().stdout_eq(str![[r#"
{
  "passed": false,
  "reasons": [
    "candidate tests failed on seed 1: Error: Compiler run failed:/nError (7576): Undeclared identifier./n [FILE]:4:9:/n  |/n4 |         missing();/n  |         ^^^^^^^",
    "candidate tests failed on seed 2: Error: Compiler run failed:/nError (7576): Undeclared identifier./n [FILE]:4:9:/n  |/n4 |         missing();/n  |         ^^^^^^^"
  ],
  "possible_bugs": []
}

"#]]).stderr_eq(str![[r#"
Error: candidate check failed

"#]]);
    cmd.forge_fuse().args(check("extra.json")).assert_failure().stdout_eq(str![[r#"
{
  "passed": false,
  "reasons": [
    "FeeRoundingTest::testHiddenRoundsUp failed on seed 1: EvmError: Revert",
    "FeeRoundingTest::testHiddenRoundsUp failed on seed 2: EvmError: Revert"
  ],
  "possible_bugs": [
    {
      "test": "FeeRoundingTest::testHiddenRoundsUp",
      "path": "test/generated/FeeCheck.t.sol",
      "reason": "seed 1: EvmError: Revert; seed 2: EvmError: Revert"
    }
  ]
}

"#]]);
    cmd.forge_fuse().args(check("invariants.json")).assert_failure().stdout_eq(str![[r#"
{
  "passed": false,
  "reasons": [
    "FeeInvariantTest::invariant_totalStaysZero failed on seed 1: assertion failed: 1 != 0; counterexample: [..]",
    "FeeInvariantTest::invariant_totalStaysZero failed on seed 2: assertion failed: 3 != 0; counterexample: [..]"
  ],
  "possible_bugs": [
    {
      "test": "FeeInvariantTest::invariant_totalStaysZero",
      "path": "test/generated/FeeCheck.t.sol",
      "reason": "seed 1: assertion failed: 1 != 0; counterexample: [..]; seed 2: assertion failed: 3 != 0; counterexample: [..]"
    }
  ]
}

"#]]);
    cmd.forge_fuse().args(check("handler.json")).assert_failure().stdout_eq(str![[r#"
{
  "passed": false,
  "reasons": [
    "HandlerInvariantTest::invariant_one failed on seed 1: handler test/generated/FeeCheck.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..]",
    "HandlerInvariantTest::invariant_one failed on seed 2: handler test/generated/FeeCheck.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..]"
  ],
  "possible_bugs": [
    {
      "test": "HandlerInvariantTest::invariant_one",
      "path": "test/generated/FeeCheck.t.sol",
      "reason": "seed 1: handler test/generated/FeeCheck.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..]; seed 2: handler test/generated/FeeCheck.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..]"
    }
  ]
}

"#]]);
    cmd.forge_fuse().args(check("seeded.json")).assert_failure().stderr_eq(str![[r#"
Error: candidate test/generated/FeeCheck.t.sol sets a seed in inline config

"#]]);
    write_candidate(
        "seeded-table.json",
        "pragma solidity ^0.8.20;\ncontract FeeCheckTest {\n    /// forge-config: default.fuzz = { seed = \"0x3\", runs = 1 }\n    function testFuzzFee(uint256 amount) public pure {}\n}\n",
    );
    cmd.forge_fuse().args(check("seeded-table.json")).assert_failure().stderr_eq(str![[r#"
Error: candidate test/generated/FeeCheck.t.sol sets a seed in inline config

"#]]);
    assert!(!prj.root().join("test/generated/FeeCheck.t.sol").exists());
    write_candidate(
        "seeded-block.json",
        "pragma solidity ^0.8.20;\ncontract FeeCheckTest {\n    /** forge-config: default.fuzz.seed = \"0x3\" */\n    function testFuzzFee(uint256 amount) public pure {}\n}\n",
    );
    cmd.forge_fuse().args(check("seeded-block.json")).assert_failure().stderr_eq(str![[r#"
Error: candidate test/generated/FeeCheck.t.sol sets a seed in inline config

"#]]);
}

#[forgetest_init]
fn properties_check_rebases_remapping_providers(prj: _, cmd: _) {
    let candidate = serde_json::json!({
        "schema": "foundry/properties-candidate-v1",
        "rationale": "check generated imports",
        "files": [
            {"path": "test/generated/Helper.sol", "content": "pragma solidity ^0.8.20; library Helper { function value() internal pure returns (uint) { return 1; } }"},
            {"path": "test/generated/Property.t.sol", "content": "pragma solidity ^0.8.20; import {Helper} from 'generated/Helper.sol'; contract Property { function testValue() public pure { require(Helper.value() == 1); } }"}
        ]
    });
    fs::write(prj.root().join("candidate.json"), candidate.to_string()).unwrap();
    let mapping = format!("generated/={}/test/generated/", prj.root().display());
    let check = ["properties", "--check", "candidate.json", "--seed", "1", "--seed", "2"];
    fs::write(prj.root().join("remappings.txt"), &mapping).unwrap();
    cmd.args(check).assert_success();
    fs::remove_file(prj.root().join("remappings.txt")).unwrap();
    for variable in ["FOUNDRY_REMAPPINGS", "DAPP_REMAPPINGS"] {
        cmd.forge_fuse().env(variable, &mapping);
        cmd.args(check).assert_success();
        cmd.unset_env(variable);
    }
}

// A last round whose tests do not compile earns one repair round with the compiler errors.
#[forgetest_init]
fn properties_repairs_a_round_that_does_not_compile(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let args = properties_args(
        &prj,
        r#"
if grep -q '^  "round": 2,' "$prompt"; then
    grep -q 'Undeclared identifier' "$prompt"
    body='require(Fee.fee(100) == 1);'
else
    body='missing();'
fi
cat > "$output" <<JSON
{"schema": "foundry/properties-candidate-v1", "rationale": "small fees are 1%", "files": [{"path": "test/generated/agent-1/FeeSpec.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeSpecTest {\n    function testSmallFee() public pure {\n        $body\n    }\n}\n"}]}
JSON
"#,
    );
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
added 1 generated test file(s):
  test/generated/agent-1/FeeSpec.t.sol

"#]]);
    assert_eq!(rounds(&prj).as_array().unwrap().len(), 2);
}

const VAULT: &str = r#"
pragma solidity ^0.8.20;

contract Vault {
    error ZeroAmount();

    mapping(address => uint256) public balanceOf;

    /// @notice Deposits `amount`. A zero amount is allowed.
    function deposit(uint256 amount) external {
        if (amount == 0) revert ZeroAmount();
        balanceOf[msg.sender] += amount;
    }

    /// @notice Returns one share per asset.
    function shares(uint256 assets) external pure returns (uint256) {
        return assets / 2;
    }
}
"#;

// By default five agents run, each with its own lens. A possible bug whose failing call a project
// test asserts in the same state contradicts that test and comes last. A kept test that its agent
// reported as a bug is a possible lock-in.
#[forgetest_init]
fn properties_checks_bugs_against_existing_tests(prj: _, cmd: _) {
    prj.add_source("Vault.sol", VAULT);
    prj.add_test(
        "Vault.t.sol",
        r#"
pragma solidity ^0.8.20;
import {Test} from "forge-std/Test.sol";
import {Vault} from "../src/Vault.sol";
contract VaultTest is Test {
    Vault internal vault;
    function setUp() public {
        vault = new Vault();
    }
    function testRejectsZero() public {
        vm.expectRevert(Vault.ZeroAmount.selector);
        vault.deposit(0);
    }
    function testDeposit() public {
        vault.deposit(5);
        assertEq(vault.balanceOf(address(this)), 5);
    }
}
"#,
    );
    let args = properties_args(
        &prj,
        r#"
agent=$(sed -n 's/^    "id": \([0-9][0-9]*\),$/\1/p' "$prompt" | head -n 1)
case "$agent" in 1|5) lens=spec ;; 2) lens=state ;; 3) lens=code ;; 4) lens=tests ;; esac
grep -q "\"lens\": \"$lens: " "$prompt"
grep -q '"count": 5' "$prompt"
grep -q '"shares": \[\]' "$prompt"
grep -q '"testRejectsZero"' "$prompt"
dir="test/generated/agent-$agent"
head='pragma solidity ^0.8.20;\nimport {Test} from \"forge-std/Test.sol\";\nimport {Vault} from \"../../../src/Vault.sol\";\n'
setup='    Vault internal vault;\n    function setUp() public {\n        vault = new Vault();\n    }\n'
zero='{"claim": "deposit(amount) accepts a zero amount.", "quote": "A zero amount is allowed.", "source": "src/Vault.sol:9", "basis": "documented", "code_path": "src/Vault.sol:11 deposit() reverts ZeroAmount."}'
shares='{"claim": "shares(assets) returns one share per asset.", "quote": "Returns one share per asset.", "source": "src/Vault.sol:15", "basis": "documented", "code_path": "src/Vault.sol:17 shares() halves the assets."}'
case "$agent" in
1|2) files="{\"path\": \"$dir/VaultBug.t.sol\", \"content\": \"${head}contract VaultBugTest is Test {\n${setup}    function testDepositZero() public {\n        vault.deposit(0);\n    }\n    function testShares() public view {\n        assertEq(vault.shares(10), 10);\n    }\n}\n\"}"
   evidence="\"VaultBugTest::testDepositZero\": $zero, \"VaultBugTest::testShares\": $shares"
   if [ "$agent" = 1 ]; then
       files="$files, {\"path\": \"$dir/VaultLock.t.sol\", \"content\": \"${head}contract VaultLockTest is Test {\n${setup}    function testZeroReverts() public {\n        vm.expectRevert(Vault.ZeroAmount.selector);\n        vault.deposit(0);\n    }\n}\n\"}"
       evidence="$evidence, \"VaultLockTest::testZeroReverts\": $zero"
   fi ;;
*) files="{\"path\": \"$dir/VaultSpec.t.sol\", \"content\": \"${head}contract VaultSpecTest is Test {\n${setup}    function testDepositFive() public {\n        vault.deposit(5);\n    }\n}\n\"}"
   evidence="" ;;
esac
cat > "$output" <<JSON
{"schema": "foundry/properties-candidate-v1", "rationale": "vault", "files": [$files], "evidence": {$evidence}}
JSON
"#,
    );
    cmd.args(args).args(["--target", "src/Vault.sol"]).assert_success().stdout_eq(str![[r#"
possible bug (documented, 2/5 agents): shares(assets) returns one share per asset.
  agent 1: VaultBugTest::testShares ([..])
    claim: shares(assets) returns one share per asset.
    quote: "Returns one share per asset." (src/Vault.sol:15)
    code path: src/Vault.sol:17 shares() halves the assets.
    fails on every seed; candidate: cache/properties/agent-1/0x[..]
  agent 2: VaultBugTest::testShares ([..])
    claim: shares(assets) returns one share per asset.
    quote: "Returns one share per asset." (src/Vault.sol:15)
    code path: src/Vault.sol:17 shares() halves the assets.
    fails on every seed; candidate: cache/properties/agent-2/0x[..]
possible bug (documented, 2/5 agents, contradicts-existing-tests): deposit(amount) accepts a zero amount.
  contradicting tests: VaultTest::testRejectsZero
  agent 1: VaultBugTest::testDepositZero (seed 24301: ZeroAmount(); seed 12648430: ZeroAmount())
    claim: deposit(amount) accepts a zero amount.
    quote: "A zero amount is allowed." (src/Vault.sol:9)
    code path: src/Vault.sol:11 deposit() reverts ZeroAmount.
    fails on every seed; candidate: cache/properties/agent-1/0x[..]
  agent 2: VaultBugTest::testDepositZero (seed 24301: ZeroAmount(); seed 12648430: ZeroAmount())
    claim: deposit(amount) accepts a zero amount.
    quote: "A zero amount is allowed." (src/Vault.sol:9)
    code path: src/Vault.sol:11 deposit() reverts ZeroAmount.
    fails on every seed; candidate: cache/properties/agent-2/0x[..]
added 2 generated test file(s):
  test/generated/agent-1/VaultLock.t.sol
  test/generated/agent-3/VaultSpec.t.sol
skipped 2 generated test file(s) that repeat a file of another agent
possible lock-in: VaultLockTest::testZeroReverts passes, but its evidence reports it as a failing test

"#]]);
}

// Forge runs journal failures again in a fresh copy of the project, because the generator can
// write the journal, and it reports project files that change while the agents run.
#[forgetest_init]
fn properties_reruns_journal_failures_and_reports_changed_files(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let args = properties_args(
        &prj,
        r#"
root=$(sed -n '/"--root",/{n;s/^ *"//;s/",*$//;p;}' "$prompt")
cat > removed.json <<'JSON'
{"schema": "foundry/properties-candidate-v1", "rationale": "cap", "files": [{"path": "test/generated/agent-1/FeeRemoved.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeRemovedTest {\n    function testDocumentedCap() public pure {\n        require(Fee.fee(1e30) == 100 ether);\n    }\n}\n"}]}
JSON
check removed.json
# A journal line that the generator wrote itself, for a test that passes.
printf '%s\n' '{"test": "FeeFakeTest::testFake", "path": "test/generated/agent-1/FeeFake.t.sol", "reason": "made up", "source": "pragma solidity ^0.8.20;\ncontract FeeFakeTest {\n    function testFake() public pure {}\n}\n"}' >> "$root/cache/properties/agent-1/check-failures.jsonl"
echo changed > "$root/notes.txt"
cat > "$output" <<'JSON'
{"schema": "foundry/properties-candidate-v1", "rationale": "small fees are 1%", "files": [{"path": "test/generated/agent-1/FeeSpec.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeSpecTest {\n    function testSmallFee() public pure {\n        require(Fee.fee(100) == 1);\n    }\n}\n"}]}
JSON
"#,
    );
    cmd.args(args).args(["--agents", "1", "--target", "src/Fee.sol"]).assert_success().stdout_eq(
        str![[r#"
possible bug (no evidence, 1/1 agents): FeeRemovedTest::testDocumentedCap
  agent 1: FeeRemovedTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    seen in a generator check, then removed; journal: cache/properties/agent-1/check-failures.jsonl
1 check journal failure(s) did not fail again in a fresh copy: see cache/properties/report.json
added 1 generated test file(s):
  test/generated/agent-1/FeeSpec.t.sol

"#]],
    )
    .stderr_eq(str![[r#"
Warning: 1 project file(s) changed while the agents ran, so the run is tainted: see cache/properties/report.json

"#]]);

    let report = fs::read_to_string(prj.root().join("cache/properties/report.json")).unwrap();
    let report = serde_json::from_str::<serde_json::Value>(&report).unwrap();
    assert_data_eq!(
        format!("{} {}", report["unverified"][0]["test"], report["tainted"]),
        str![[r#""FeeFakeTest::testFake" ["notes.txt"]"#]]
    );
}

/// Writes a candidate with one test file at `test/generated/Stub.t.sol` and returns its path.
fn write_stub_candidate(prj: &TestProject, name: &str, content: &str) -> String {
    let candidate = serde_json::json!({
        "schema": "foundry/properties-candidate-v1",
        "rationale": "stub",
        "files": [{"path": "test/generated/Stub.t.sol", "content": content}]
    });
    fs::write(prj.root().join(name), candidate.to_string()).unwrap();
    name.to_string()
}

// A pnpm package is a link into `node_modules/.pnpm`, and it imports its own files by absolute
// paths such as `src/lib/Init.sol`. The workspace must resolve them as the project does.
#[forgetest_init]
fn properties_check_resolves_pnpm_package_imports(prj: _, cmd: _) {
    let package = prj.root().join("node_modules/.pnpm/pkg@1.0.0/node_modules/pkg");
    fs::create_dir_all(package.join("src/lib")).unwrap();
    fs::create_dir_all(package.join("src/utils")).unwrap();
    fs::write(
        package.join("src/lib/Init.sol"),
        "pragma solidity ^0.8.20;\nlibrary Init {\n    function value() internal pure returns (uint256) {\n        return 7;\n    }\n}\n",
    )
    .unwrap();
    fs::write(
        package.join("src/utils/Proxy.sol"),
        "pragma solidity ^0.8.20;\nimport {Init} from \"src/lib/Init.sol\";\ncontract Proxy {\n    function value() external pure returns (uint256) {\n        return Init.value();\n    }\n}\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(
        ".pnpm/pkg@1.0.0/node_modules/pkg",
        prj.root().join("node_modules/pkg"),
    )
    .unwrap();
    prj.update_config(|config| {
        config.libs.push("node_modules".into());
        config.remappings = vec![Remapping::from_str("pkg/=node_modules/pkg/src/").unwrap().into()];
    });
    let candidate = write_stub_candidate(
        &prj,
        "candidate.json",
        "pragma solidity ^0.8.20;\nimport {Proxy} from \"pkg/utils/Proxy.sol\";\ncontract StubTest {\n    function testValue() public {\n        require(new Proxy().value() == 7);\n    }\n}\n",
    );
    cmd.args(["properties", "--check", &candidate, "--seed", "1", "--seed", "2"])
        .assert_success()
        .stdout_eq(str![[r#"
{
  "passed": true,
  "reasons": [],
  "possible_bugs": []
}

"#]]);
}

// A project test imports a file from a directory that is not a source, test, script, or library
// directory. The workspace must still have it.
#[forgetest_init]
fn properties_check_copies_other_project_directories(prj: _, cmd: _) {
    fs::create_dir_all(prj.root().join("deployments/genesis")).unwrap();
    fs::write(
        prj.root().join("deployments/genesis/Genesis.sol"),
        "pragma solidity ^0.8.20;\nlibrary Genesis {\n    function chainId() internal pure returns (uint256) {\n        return 2017;\n    }\n}\n",
    )
    .unwrap();
    prj.add_test(
        "utils/Helper.sol",
        "pragma solidity ^0.8.20;\nimport {Genesis} from \"deployments/genesis/Genesis.sol\";\nlibrary Helper {\n    function chainId() internal pure returns (uint256) {\n        return Genesis.chainId();\n    }\n}\n",
    );
    let candidate = write_stub_candidate(
        &prj,
        "candidate.json",
        "pragma solidity ^0.8.20;\nimport {Helper} from \"../utils/Helper.sol\";\ncontract StubTest {\n    function testChainId() public pure {\n        require(Helper.chainId() == 2017);\n    }\n}\n",
    );
    cmd.args(["properties", "--check", &candidate, "--seed", "1", "--seed", "2"])
        .assert_success()
        .stdout_eq(str![[r#"
{
  "passed": true,
  "reasons": [],
  "possible_bugs": []
}

"#]]);
}
