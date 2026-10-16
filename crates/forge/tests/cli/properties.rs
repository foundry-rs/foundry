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

const ARITHMETIC: &str = r#"
pragma solidity ^0.8.20;

contract Arithmetic {
    function bucket(uint256 value) external pure returns (uint256) {
        if (value < 10) return 1;
        if (value < 100) return 2;
        return 3;
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

/// Returns `cache/properties/rounds.json`.
fn rounds(prj: &TestProject) -> serde_json::Value {
    let rounds = fs::read_to_string(prj.root().join("cache/properties/rounds.json")).unwrap();
    serde_json::from_str(&rounds).unwrap()
}

// The audit reports the failing tests with their evidence, documented first, and keeps the files
// whose tests pass. Without targets, the prompt has every source file.
#[forgetest_init]
fn properties_reports_possible_bugs_by_evidence(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let context = prj.root().join("context.md");
    fs::write(&context, "context-marker: rounding notes").unwrap();
    let mut args = properties_args(
        &prj,
        r#"
grep -q '^  "phase": "audit"' "$prompt"
grep -q 'audit first, test second' "$prompt"
grep -q '"path": "src/Fee.sol"' "$prompt"
grep -q 'context-marker: rounding notes' "$prompt"
if grep -q '"survivors"' "$prompt"; then exit 1; fi
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "cap: finding; charges: finding; rounding: finding; small fee: refuted",
  "files": [
    {"path": "test/generated/FeeAudit.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../src/Fee.sol\";\ncontract FeeAuditTest {\n    function testDocumentedCap() public pure {\n        require(Fee.fee(1e30) == 100 ether);\n    }\n    function testChargesZero() public pure {\n        require(Fee.charges(0));\n    }\n    function testRoundsUp() public pure {\n        require(Fee.fee(199) == 2);\n    }\n}\n"},
    {"path": "test/generated/FeeSpec.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../src/Fee.sol\";\ncontract FeeSpecTest {\n    function testSmallFee() public pure {\n        require(Fee.fee(100) == 1);\n    }\n}\n"}
  ],
  "evidence": {
    "FeeAuditTest::testChargesZero": {
      "claim": "A zero amount is charged.",
      "quote": "",
      "source": "the name of charges",
      "basis": "inferred",
      "code_path": "src/Fee.sol:11 returns false for amounts below 100."
    },
    "FeeAuditTest::testDocumentedCap": {
      "claim": "The fee is capped at 100 ether.",
      "quote": "Charges 1% of `amount`, capped at 100 ether.",
      "source": "src/Fee.sol:3",
      "basis": "documented",
      "code_path": "src/Fee.sol:7 caps the fee at 10 ether."
    }
  }
}
JSON
# A checked failure that the generator keeps is reported once.
check "$output"
"#,
    );
    args.extend(["--context".to_string(), context.to_str().unwrap().to_string()]);
    cmd.args(args).assert_success().stdout_eq(str![[r#"
possible bug (documented): FeeAuditTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
  claim: The fee is capped at 100 ether.
  quote: "Charges 1% of `amount`, capped at 100 ether." (src/Fee.sol:3)
  code path: src/Fee.sol:7 caps the fee at 10 ether.
  fails on every seed; candidate: cache/properties/0x[..]
possible bug (inferred): FeeAuditTest::testChargesZero (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
  claim: A zero amount is charged.
  quote: "" (the name of charges)
  code path: src/Fee.sol:11 returns false for amounts below 100.
  fails on every seed; candidate: cache/properties/0x[..]
possible bug (no evidence): FeeAuditTest::testRoundsUp (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
  fails on every seed; candidate: cache/properties/0x[..]
added 1 generated test file(s):
  test/generated/FeeSpec.t.sol

"#]]);

    assert!(!prj.root().join("test/generated/FeeAudit.t.sol").exists());
    assert!(!prj.root().join("cache/properties/baseline").exists());
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
    {"path": "test/generated/Helper.sol", "content": "pragma solidity ^0.8.20;\nlibrary Helper { function fee(uint256 amount) internal pure returns (uint256) { return amount / 100; } }\n"},
    {"path": "test/generated/Property.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../src/Fee.sol\";\nimport {Helper} from \"./Helper.sol\";\ncontract PropertyTest { function testFee() public pure { require(Fee.fee(199) == Helper.fee(199)); } }\n"},
    {"path": "test/generated/Unused.sol", "content": "pragma solidity ^0.8.20;\nlibrary Unused {}\n"}
  ]
}
JSON
"#,
    );
    cmd.args(args).args(["--target", "src/Fee.sol"]).assert_success().stdout_eq(str![[r#"
added 2 generated test file(s):
  test/generated/Helper.sol
  test/generated/Property.t.sol

"#]]);

    assert!(!prj.root().join("test/generated/Unused.sol").exists());
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
  "files": [{"path": "test/generated/FeeRemoved.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../src/Fee.sol\";\ncontract FeeRemovedTest {\n    function testDocumentedCap() public pure {\n        require(Fee.fee(1e30) == 100 ether);\n    }\n}\n"}]
}
JSON
check removed.json
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "small fees are 1%",
  "files": [{"path": "test/generated/FeeSpec.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../src/Fee.sol\";\ncontract FeeSpecTest {\n    function testSmallFee() public pure {\n        require(Fee.fee(100) == 1);\n    }\n}\n"}]
}
JSON
"#,
    );
    cmd.args(args)
        .args(["--target", "src/Fee.sol", "--rounds", "2"])
        .assert_failure()
        .stdout_eq(str![[r#"
possible bug (no evidence): FeeRemovedTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
  seen in a generator check, then removed; journal: cache/properties/check-failures.jsonl
added 1 generated test file(s):
  test/generated/FeeSpec.t.sol

"#]])
        .stderr_eq(str![[r#"
Error: generator failed in round 2 (exit status: 3): generator quota exceeded

"#]]);

    // The journal keeps the removed test's source for review.
    let journal =
        fs::read_to_string(prj.root().join("cache/properties/check-failures.jsonl")).unwrap();
    let journal = serde_json::from_str::<serde_json::Value>(&journal).unwrap();
    assert!(journal["source"].as_str().unwrap().contains("function testDocumentedCap()"));
}

// After the audit, `--mutation` keeps a new file only if it kills a survivor on every seed. The
// mutation runs select the generated tests although configured filters exclude them.
#[forgetest_init]
fn properties_mutation_keeps_survivor_killing_tests(prj: _, cmd: _) {
    prj.add_source("Arithmetic.sol", ARITHMETIC);
    prj.add_test(
        "Arithmetic.t.sol",
        r#"
pragma solidity ^0.8.20;

import {Arithmetic} from "../src/Arithmetic.sol";

contract ArithmeticTest {
    Arithmetic internal arithmetic = new Arithmetic();

    function testSmallValue() public view {
        require(arithmetic.bucket(1) == 1);
    }
}
"#,
    );
    prj.update_config(|config| {
        config.test_pattern = Some(regex::Regex::new(r"^testSmall\w*\(").unwrap().into());
    });
    let file = |name: &str, body: &str| {
        format!(
            r#"cat > "$output" <<'JSON'
{{"schema": "foundry/properties-candidate-v1", "rationale": "{name}", "files": [{{"path": "test/generated/{name}.t.sol", "content": "pragma solidity ^0.8.20;\nimport {{Arithmetic}} from \"../../src/Arithmetic.sol\";\ncontract {name}Test {{\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testProperty() public view {{\n        {body}\n    }}\n}}\n"}}]}}
JSON
exit 0
"#
        )
    };
    let script = format!(
        r#"
if grep -q '^  "phase": "audit"' "$prompt"; then
    if grep -q '"survivors"' "$prompt"; then exit 1; fi
    if grep -q '^  "round": 1,' "$prompt"; then
{}    fi
{}fi
grep -q '^  "phase": "mutation"' "$prompt"
grep -q '"survives_all_seeds": true' "$prompt"
grep -q '"source_context"' "$prompt"
if grep -q '^  "round": 3,' "$prompt"; then
{}fi
grep -q 'ArithmeticNoGainTest' "$prompt"
{}"#,
        file("ArithmeticLarge", r"require(arithmetic.bucket(1000) == 3);"),
        file("ArithmeticMiddle", r"require(arithmetic.bucket(50) == 2);"),
        file("ArithmeticNoGain", r"require(arithmetic.bucket(1) == 1);"),
        file(
            "ArithmeticLower",
            r"require(arithmetic.bucket(9) == 1); require(arithmetic.bucket(10) == 2);"
        ),
    );
    cmd.args(properties_args(&prj, &script))
        .args(["--target", "src/Arithmetic.sol", "--rounds", "2", "--mutation"])
        .args(["--match-contract", "^ArithmeticTest$"])
        .assert_success()
        .stdout_eq(str![[r#"
added 3 generated test file(s):
  test/generated/ArithmeticLarge.t.sol
  test/generated/ArithmeticMiddle.t.sol
  test/generated/ArithmeticLower.t.sol
the mutation phase killed 1 survivor(s); 1 remain

"#]]);

    let rounds = rounds(&prj);
    assert_data_eq!(
        serde_json::to_string_pretty(&rounds[2]["reasons"]).unwrap(),
        str![[r#"
[
  "the passing files kill no surviving mutant on every seed"
]
"#]]
    );
    assert!(rounds[3]["killed_survivors"].as_u64().unwrap() > 0);
    assert!(!prj.root().join("test/generated/ArithmeticNoGain.t.sol").exists());
}

// The target is in a linked dependency directory, so mutation runs must copy it into the nested
// workspace. It imports a library through a remapping and the test imports the library by its
// project path; both must name the same file there, or the types differ.
#[forgetest_init]
fn properties_keeps_library_paths_in_nested_workspaces(prj: _, cmd: _) {
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
if grep -q '^  "phase": "mutation"' "$prompt"; then
    name=Upper
    body='require(arithmetic.bucket(99) == 2); require(arithmetic.bucket(100) == 3);'
fi
cat > "$output" <<JSON
{"schema": "foundry/properties-candidate-v1", "rationale": "$name", "files": [{"path": "test/generated/Arithmetic$name.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Arithmetic} from \"bucket/Arithmetic.sol\";\ncontract Arithmetic${name}Test {\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testBoundary() public view {\n        $body\n    }\n}\n"}]}
JSON
"#,
    );
    cmd.args(args)
        .args(["--target", "node_modules/bucket/Arithmetic.sol", "--mutation"])
        .args(["--match-contract", "^ArithmeticTest$"])
        .assert_success()
        .stdout_eq(str![[r#"
added 2 generated test file(s):
  test/generated/ArithmeticLower.t.sol
  test/generated/ArithmeticUpper.t.sol
the mutation phase killed 2 survivor(s); 3 remain

"#]]);
}

// A generated file must not leave the project through a symlinked directory.
#[forgetest_init]
fn properties_rejects_symlinked_generated_directory(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(prj.root().join("test/generated")).unwrap();
    std::os::unix::fs::symlink(outside.path(), prj.root().join("test/generated/link")).unwrap();
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
  "files": [{"path": "test/generated/link/FeeSpec.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../../src/Fee.sol\";\ncontract FeeSpecTest {\n    function testSmallFee() public pure {\n        require(Fee.fee(100) == 1);\n    }\n}\n"}]
}
JSON
"#,
    );
    cmd.args(args).assert_success().stdout_eq(str![[r#"
no generated test file was kept

"#]]);

    assert_data_eq!(
        rounds(&prj)[0]["reasons"][0].as_str().unwrap(),
        str!["generated test path test/generated/link/FeeSpec.t.sol escapes project root [..]"]
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
