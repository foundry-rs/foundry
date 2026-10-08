//! Tests for the `forge properties` command.

use foundry_test_utils::{assert_data_eq, forgetest_init, str};
use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
#[forgetest_init]
fn properties_retains_reproducible_property(prj: _, cmd: _) {
    let assertion_lib = prj.root().join("lib/example");
    fs::create_dir_all(&assertion_lib).unwrap();
    fs::write(
        assertion_lib.join("Assertions.sol"),
        r#"
pragma solidity ^0.8.20;

library Assertions {
    function equal(uint256 left, uint256 right) internal pure {
        assert(left == right);
    }
}
"#,
    )
    .unwrap();
    prj.add_source(
        "Arithmetic.sol",
        r#"
pragma solidity ^0.8.20;

contract Arithmetic {
    function bucket(uint256 value) external pure returns (uint256) {
        if (value < 10) return 1;
        if (value < 100) return 2;
        return 3;
    }
}
"#,
    );
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
    let test_path = prj.root().join("test/Arithmetic.t.sol");
    let mut test_source = fs::read_to_string(&test_path).unwrap();
    test_source.push_str("/*x");
    test_source.push_str(&"€".repeat(6_000));
    test_source.push_str("*/\n");
    fs::write(test_path, test_source).unwrap();
    fs::rename(prj.root().join("test"), prj.root().join("tests")).unwrap();
    prj.update_config(|config| {
        config.test = "tests".into();
        config.remappings = vec![
            "example/=lib/example/"
                .parse::<foundry_compilers::artifacts::remappings::Remapping>()
                .unwrap()
                .into(),
        ];
    });
    fs::create_dir_all(prj.root().join("tests/generated")).unwrap();
    fs::write(
        prj.root().join("tests/generated/Existing.t.sol"),
        "pragma solidity ^0.8.20;\nimport {Arithmetic} from \"../../src/Arithmetic.sol\";\n",
    )
    .unwrap();

    let brief = prj.root().join("brief.md");
    fs::write(&brief, "Exercise every bucket boundary.").unwrap();
    let generator = prj.root().join("generator.sh");
    fs::write(
        &generator,
        r#"#!/bin/sh
set -eu
prompt="$1"
output="$2"
grep -q '"mutation_gaps"' "$prompt"
grep -q '"current_results"' "$prompt"
grep -q '"original"' "$prompt"
grep -q '"mutant"' "$prompt"
grep -q '"survives_all_seeds": true' "$prompt"
grep -q '"source_context"' "$prompt"
grep -q 'function bucket' "$prompt"
grep -q 'mutants may be semantically equivalent' "$prompt"
grep -q 'example/=lib/example/' "$prompt"
grep -q '"path": "tests/Arithmetic.t.sol"' "$prompt"
grep -q 'contract ArithmeticTest' "$prompt"
grep -q '"truncated": true' "$prompt"
if grep -q '"round": 1' "$prompt" && grep -q '"last_rejected_sources"' "$prompt"; then
    exit 1
fi
if grep -q '"round": 1' "$prompt" &&
    grep -q '"path": "tests/generated/Existing.t.sol"' "$prompt"; then
    exit 1
fi
if ! grep -q '"round": 1' "$prompt"; then grep -q '"candidate_results"' "$prompt"; fi
if grep -q '"round": 1' "$prompt"; then
grep -q 'contract GeneratedRound1Test' "$prompt"
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "exercise candidate rejection",
  "files": [{"path": "tests/generated/Existing.t.sol", "content": "pragma solidity ^0.8.20;\n// rejected-overwrite-marker\n"}]
}
JSON
exit 0
fi
if grep -q '"round": 2' "$prompt"; then
grep -q '"last_rejected_sources"' "$prompt"
grep -q 'rejected-overwrite-marker' "$prompt"
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "duplicate the existing small-value example",
  "files": [{
    "path": "tests/generated/ArithmeticNoGain.t.sol",
    "content": "pragma solidity ^0.8.20;\nimport {Arithmetic} from \"../../src/Arithmetic.sol\";\n// rejected-no-gain-marker\ncontract ArithmeticNoGainTest {\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testSmallValue() public view {\n        require(arithmetic.bucket(1) == 1);\n    }\n}\n"
  }]
}
JSON
exit 0
fi
if grep -q '"round": 3' "$prompt"; then
grep -q '"last_rejected_sources"' "$prompt"
grep -q 'rejected-no-gain-marker' "$prompt"
if grep -q 'rejected-overwrite-marker' "$prompt"; then exit 1; fi
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "exercise the lower comparison boundary",
  "generator": {"agent": "fixture", "model": "deterministic"},
  "files": [{
    "path": "tests/generated/ArithmeticLower.t.sol",
    "content": "pragma solidity ^0.8.20;\nimport {Assertions} from \"../../lib/example/Assertions.sol\";\nimport {Arithmetic} from \"../../src/Arithmetic.sol\";\ncontract ArithmeticLowerTest {\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testLowerBoundary() public view {\n        Assertions.equal(arithmetic.bucket(9), 1);\n        Assertions.equal(arithmetic.bucket(10), 2);\n    }\n}\n"
  }]
}
JSON
exit 0
fi
grep -q '"current_candidate"' "$prompt"
grep -q 'ArithmeticLowerTest' "$prompt"
if grep -q '"round": 4' "$prompt"; then
if grep -q '"last_rejected_sources"' "$prompt"; then exit 1; fi
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "try to replace an accepted property",
  "files": [{"path": "tests/generated/ArithmeticLower.t.sol", "content": "pragma solidity ^0.8.20;\n// rejected-retained-path-marker\n"}]
}
JSON
exit 0
fi
grep -q '"last_rejected_sources"' "$prompt"
grep -q 'rejected-retained-path-marker' "$prompt"
if grep -q 'rejected-no-gain-marker' "$prompt"; then exit 1; fi
cat > "$output" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "exercise the upper comparison boundary",
  "generator": {"agent": "fixture", "model": "deterministic"},
  "files": [{
    "path": "tests/generated/ArithmeticUpper.t.sol",
    "content": "pragma solidity ^0.8.20;\nimport {Assertions} from \"../../lib/example/Assertions.sol\";\nimport {Arithmetic} from \"../../src/Arithmetic.sol\";\ncontract ArithmeticUpperTest {\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testUpperBoundary() public view {\n        Assertions.equal(arithmetic.bucket(99), 2);\n        Assertions.equal(arithmetic.bucket(100), 3);\n    }\n}\n"
  }]
}
JSON
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&generator).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&generator, permissions).unwrap();

    cmd.args([
        "properties",
        "--root",
        prj.root().to_str().unwrap(),
        "--mutate",
        "src/Arithmetic.sol",
        "--brief",
        brief.to_str().unwrap(),
        "--generator",
        generator.to_str().unwrap(),
        "--seed",
        "0x5eed",
        "--seed",
        "0xc0ffee",
        "--match-contract",
        "^ArithmeticTest$",
        "--rounds",
        "5",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
added 2 generated test file(s) that reproducibly resolve 6 mutation survivor(s):
  tests/generated/ArithmeticLower.t.sol
  tests/generated/ArithmeticUpper.t.sol

"#]]);

    let rounds = fs::read_to_string(prj.root().join("cache/properties/rounds.json")).unwrap();
    assert!(!rounds.contains("\"mutant\""));
    let rounds: serde_json::Value = serde_json::from_str(&rounds).unwrap();
    assert_eq!(rounds.as_array().unwrap().len(), 5);
    assert_eq!(rounds[0]["accepted"], false);
    assert!(rounds[0]["reasons"][0].as_str().unwrap().contains("would overwrite"));
    assert_eq!(rounds[1]["accepted"], false);
    assert_eq!(rounds[1]["candidate"].as_array().unwrap().len(), 2);
    assert!(
        rounds[1]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("did not reproducibly resolve a current mutation survivor")
    );
    assert_eq!(rounds[2]["accepted"], true);
    assert_eq!(rounds[2]["generator"]["model"], "deterministic");
    assert!(rounds[2]["resolved_survivors"].as_u64().unwrap() > 0);
    assert_eq!(rounds[2]["resolved_survivors"], rounds[2]["newly_resolved_survivors"]);
    assert_eq!(rounds[3]["accepted"], false);
    assert!(rounds[3]["reasons"][0].as_str().unwrap().contains("retained by an earlier round"));
    assert_eq!(rounds[4]["accepted"], true);
    assert!(rounds[4]["resolved_survivors"].as_u64().unwrap() > 0);
    assert!(rounds[4]["newly_resolved_survivors"].as_u64().unwrap() > 0);

    let digest = rounds[4]["candidate_digest"].as_str().unwrap();
    let candidate =
        fs::read_to_string(prj.root().join("cache/properties").join(digest).join("candidate.json"))
            .unwrap();
    assert!(candidate.contains("ArithmeticLowerTest"));
    assert!(candidate.contains("ArithmeticUpperTest"));
    for name in ["ArithmeticLower", "ArithmeticUpper"] {
        let source =
            fs::read_to_string(prj.root().join(format!("tests/generated/{name}.t.sol"))).unwrap();
        assert!(source.contains(&format!("contract {name}Test")));
    }
}

#[cfg(unix)]
#[forgetest_init]
fn properties_materializes_node_modules_for_candidate_mutations(prj: _, cmd: _) {
    const GENERATOR: &str = r#"#!/bin/sh
set -eu
cat > "$2" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "exercise the lower comparison boundary",
  "files": [{"path": "test/generated/ArithmeticLower.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Assertions} from \"example/Assertions.sol\";\nimport {Arithmetic} from \"bucket/Arithmetic.sol\";\ncontract ArithmeticLowerTest {\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testLowerBoundary() public view {\n        Assertions.equal(arithmetic.bucket(9), 1); Assertions.equal(arithmetic.bucket(10), 2);\n    }\n}\n"}]
}
JSON
"#;

    prj.add_test(
        "Arithmetic.t.sol",
        r#"
pragma solidity ^0.8.20;

import {Arithmetic} from "bucket/Arithmetic.sol";

contract ArithmeticTest {
    Arithmetic internal arithmetic = new Arithmetic();

    function testSmallValue() public view {
        require(arithmetic.bucket(1) == 1);
    }
}
"#,
    );
    let brief = prj.root().join("brief.md");
    fs::write(&brief, "Exercise every bucket boundary.").unwrap();
    // The mutation target is a dependency: candidate workspaces must copy it, not link it.
    fs::create_dir_all(prj.root().join("node_modules/bucket")).unwrap();
    fs::write(
        prj.root().join("node_modules/bucket/Arithmetic.sol"),
        "pragma solidity ^0.8.20;\ncontract Arithmetic {\n    function bucket(uint256 value) external pure returns (uint256) {\n        if (value < 10) return 1;\n        if (value < 100) return 2;\n        return 3;\n    }\n}\n",
    )
    .unwrap();
    fs::create_dir_all(prj.root().join("node_modules/example")).unwrap();
    fs::write(
        prj.root().join("node_modules/example/Assertions.sol"),
        "pragma solidity ^0.8.20;\nlibrary Assertions {\n    function equal(uint256 a, uint256 b) internal pure {\n        assert(a == b);\n    }\n}\n",
    )
    .unwrap();
    prj.update_config(|config| {
        config.remappings = ["bucket/=node_modules/bucket/", "example/=node_modules/example/"]
            .map(|remapping| {
                remapping
                    .parse::<foundry_compilers::artifacts::remappings::Remapping>()
                    .unwrap()
                    .into()
            })
            .to_vec();
    });
    let generator = prj.root().join("generator.sh");
    fs::write(&generator, GENERATOR).unwrap();
    let mut permissions = fs::metadata(&generator).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&generator, permissions).unwrap();

    cmd.args([
        "properties",
        "--root",
        prj.root().to_str().unwrap(),
        "--mutate",
        "node_modules/bucket/Arithmetic.sol",
        "--brief",
        brief.to_str().unwrap(),
        "--generator",
        generator.to_str().unwrap(),
        "--seed",
        "0x5eed",
        "--seed",
        "0xc0ffee",
        "--match-contract",
        "^ArithmeticTest$",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
added 1 generated test file(s) that reproducibly resolve [..] mutation survivor(s):
  test/generated/ArithmeticLower.t.sol

"#]]);
}

#[cfg(unix)]
#[forgetest_init]
fn properties_mutation_selection_includes_generated_tests(prj: _, cmd: _) {
    const GENERATOR: &str = r#"#!/bin/sh
set -eu
cat > "$2" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "exercise the lower comparison boundary",
  "files": [{"path": "test/generated/ArithmeticLower.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Arithmetic} from \"../../src/Arithmetic.sol\";\ncontract ArithmeticLowerTest {\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testLowerBoundary() public view {\n        require(arithmetic.bucket(9) == 1); require(arithmetic.bucket(10) == 2);\n    }\n    function testUpperBoundary() public pure {}\n}\n"}]
}
JSON
"#;

    prj.add_source(
        "Arithmetic.sol",
        r#"
pragma solidity ^0.8.20;

contract Arithmetic {
    function bucket(uint256 value) external pure returns (uint256) {
        if (value < 10) return 1;
        if (value < 100) return 2;
        return 3;
    }
}
"#,
    );
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

    function testUpperBoundary() public view {
        require(arithmetic.bucket(99) == 2);
        require(arithmetic.bucket(100) == 3);
    }
}
"#,
    );
    let brief = prj.root().join("brief.md");
    fs::write(&brief, "Exercise every bucket boundary.").unwrap();
    // Configured filters exclude the generated test; candidate mutation runs must still select it.
    // They must not select `ArithmeticTest::testUpperBoundary`, which shares a generated test name.
    prj.update_config(|config| {
        config.test_pattern = Some(regex::Regex::new(r"^testSmall\w*\(").unwrap().into());
        config.path_pattern = Some("**/Arithmetic.t.sol".parse().unwrap());
    });
    let generator = prj.root().join("generator.sh");
    fs::write(&generator, GENERATOR).unwrap();
    let mut permissions = fs::metadata(&generator).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&generator, permissions).unwrap();

    cmd.args([
        "properties",
        "--root",
        prj.root().to_str().unwrap(),
        "--mutate",
        "src/Arithmetic.sol",
        "--brief",
        brief.to_str().unwrap(),
        "--generator",
        generator.to_str().unwrap(),
        "--seed",
        "0x5eed",
        "--seed",
        "0xc0ffee",
        "--match-contract",
        "^ArithmeticTest$",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
added 1 generated test file(s) that reproducibly resolve 4 mutation survivor(s):
  test/generated/ArithmeticLower.t.sol

"#]]);
}

#[cfg(unix)]
#[forgetest_init]
fn properties_rejects_symlinked_generated_directory(prj: _, cmd: _) {
    const GENERATOR: &str = r#"#!/bin/sh
set -eu
cat > "$2" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "exercise the lower comparison boundary",
  "files": [{"path": "test/generated/link/ArithmeticLower.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Arithmetic} from \"../../../src/Arithmetic.sol\";\ncontract ArithmeticLowerTest {\n    Arithmetic internal arithmetic = new Arithmetic();\n    function testLowerBoundary() public view {\n        require(arithmetic.bucket(9) == 1); require(arithmetic.bucket(10) == 2);\n    }\n}\n"}]
}
JSON
"#;

    prj.add_source(
        "Arithmetic.sol",
        r#"
pragma solidity ^0.8.20;

contract Arithmetic {
    function bucket(uint256 value) external pure returns (uint256) {
        if (value < 10) return 1;
        if (value < 100) return 2;
        return 3;
    }
}
"#,
    );
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
    let brief = prj.root().join("brief.md");
    fs::write(&brief, "Exercise every bucket boundary.").unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(prj.root().join("test/generated")).unwrap();
    std::os::unix::fs::symlink(outside.path(), prj.root().join("test/generated/link")).unwrap();
    // Internal runs keep their own `--rerun` state; the project's file stays as it is.
    let failures = prj.root().join("cache/test-failures");
    fs::create_dir_all(failures.parent().unwrap()).unwrap();
    fs::write(&failures, r#"{"version":1,"failures":[]}"#).unwrap();
    let generator = prj.root().join("generator.sh");
    fs::write(&generator, GENERATOR).unwrap();
    let mut permissions = fs::metadata(&generator).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&generator, permissions).unwrap();

    cmd.args([
        "properties",
        "--root",
        prj.root().to_str().unwrap(),
        "--mutate",
        "src/Arithmetic.sol",
        "--brief",
        brief.to_str().unwrap(),
        "--generator",
        generator.to_str().unwrap(),
        "--seed",
        "0x5eed",
        "--seed",
        "0xc0ffee",
        "--match-contract",
        "^ArithmeticTest$",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
no candidate reproducibly resolved a mutation survivor

"#]]);

    let rounds: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(prj.root().join("cache/properties/rounds.json")).unwrap(),
    )
    .unwrap();
    assert_data_eq!(
        rounds[0]["reasons"][0].as_str().unwrap(),
        str![
            "generated test path test/generated/link/ArithmeticLower.t.sol escapes project root [..]"
        ]
    );
    assert!(!outside.path().join("ArithmeticLower.t.sol").exists());
    assert_eq!(fs::read_to_string(&failures).unwrap(), r#"{"version":1,"failures":[]}"#);
}

#[cfg(unix)]
#[forgetest_init]
fn properties_reports_possible_bugs(prj: _, cmd: _) {
    const GENERATOR: &str = r#"#!/bin/sh
set -eu
if grep -q '"round": 3' "$1"; then
    echo "generator quota exceeded" >&2
    exit 3
fi
if grep -q '"round": 2' "$1"; then
grep -q '"possible_bugs"' "$1"
cat > "$2" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "large fees are capped at the current cap",
  "files": [{"path": "test/generated/FeeCurrentCap.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../src/Fee.sol\";\ncontract FeeCurrentCapTest {\n    function testCurrentCap() public pure {\n        require(Fee.fee(1e30) == 10 ether);\n        require(Fee.fee(1001 ether) == 10 ether);\n    }\n}\n"}]
}
JSON
exit 0
fi
grep -q '"likely_equivalent": "the comparisons agree when the operand is unsigned"' "$1"
grep -q '"check_command"' "$1"
grep -q 'CANDIDATE_JSON' "$1"
cat > "$2" <<'JSON'
{
  "schema": "foundry/properties-candidate-v1",
  "rationale": "the documented cap is 100 ether",
  "files": [{"path": "test/generated/FeeCap.t.sol", "content": "pragma solidity ^0.8.20;\nimport {Fee} from \"../../src/Fee.sol\";\ncontract FeeCapTest {\n    function testFuzzCap(uint256 amount) public pure {\n        amount = amount % 1e30;\n        uint256 expected = amount / 100;\n        if (expected > 100 ether) expected = 100 ether;\n        require(Fee.fee(amount) == expected, \"fee\");\n    }\n}\n"}]
}
JSON
"#;

    prj.add_source(
        "Fee.sol",
        r#"
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
"#,
    );
    prj.add_test(
        "Fee.t.sol",
        r#"
pragma solidity ^0.8.20;

import {Fee} from "../src/Fee.sol";

contract FeeTest {
    function testSmallFee() public pure {
        require(Fee.fee(100) == 1);
    }
}
"#,
    );
    let brief = prj.root().join("brief.md");
    fs::write(&brief, "Check the documented fee cap.").unwrap();
    let generator = prj.root().join("generator.sh");
    fs::write(&generator, GENERATOR).unwrap();
    let mut permissions = fs::metadata(&generator).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&generator, permissions).unwrap();

    cmd.args([
        "properties",
        "--root",
        prj.root().to_str().unwrap(),
        "--mutate",
        "src/Fee.sol",
        "--brief",
        brief.to_str().unwrap(),
        "--generator",
        generator.to_str().unwrap(),
        "--seed",
        "0x5eed",
        "--seed",
        "0xc0ffee",
        "--match-contract",
        "^FeeTest$",
        "--rounds",
        "3",
    ])
    .assert_failure()
    .stdout_eq(str![[r#"
possible bug: FeeCapTest::testFuzzCap (seed 24301: fee; counterexample: [..]; seed 12648430: fee; counterexample: [..])
  the property fails on every seed against the current implementation; candidate: cache/properties/0x[..]
added 1 generated test file(s) that reproducibly resolve [..] mutation survivor(s):
  test/generated/FeeCurrentCap.t.sol (kept after a possible bug was reported; review it together with that report)

"#]])
    .stderr_eq(str![[r#"
Error: generator failed in round 3 (exit status: 3): generator quota exceeded

"#]]);

    let rounds: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(prj.root().join("cache/properties/rounds.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(rounds.as_array().unwrap().len(), 2);
    assert_eq!(rounds[0]["possible_bugs"].as_array().unwrap().len(), 1);
    assert_eq!(rounds[1]["after_possible_bug"], true);
    assert!(!prj.root().join("test/generated/FeeCap.t.sol").exists());
}

#[cfg(unix)]
#[forgetest_init]
fn properties_check_reports_candidate_result(prj: _, cmd: _) {
    // Candidate checks run in a copy of the project; inherited config and negative filters must
    // still apply as in the project, and must not hide generated tests.
    fs::write(
        prj.root().join("base.toml"),
        "[profile.default]\nremappings = [\"fee/=contracts/\"]\n",
    )
    .unwrap();
    // A source directory other than `src` checks that library remappings, such as forge-std's,
    // keep resolving in the copy.
    prj.update_config(|config| {
        config.src = "contracts".into();
        config.test_pattern_inverse = Some(regex::Regex::new(r"^testHidden\w*").unwrap().into());
    });
    let foundry_toml = prj.root().join("foundry.toml");
    let toml = fs::read_to_string(&foundry_toml).unwrap();
    fs::write(
        &foundry_toml,
        toml.replacen("[profile.default]\n", "[profile.default]\nextends = \"base.toml\"\n", 1),
    )
    .unwrap();
    prj.add_source(
        "Fee.sol",
        r#"
pragma solidity ^0.8.20;

library Fee {
    function fee(uint256 amount) internal pure returns (uint256) {
        return amount / 100;
    }
}
"#,
    );
    fs::create_dir_all(prj.root().join("contracts")).unwrap();
    fs::rename(prj.root().join("src/Fee.sol"), prj.root().join("contracts/Fee.sol")).unwrap();
    let candidate = |content: &str| {
        serde_json::json!({
            "schema": "foundry/properties-candidate-v1",
            "rationale": "fee is 1% rounded down",
            "files": [{"path": "test/generated/FeeCheck.t.sol", "content": content}]
        })
        .to_string()
    };
    fs::write(
        prj.root().join("passing.json"),
        candidate(
            "pragma solidity ^0.8.20;\nimport {Fee} from \"fee/Fee.sol\";\ncontract FeeCheckTest {\n    function testFee() public pure {\n        require(Fee.fee(199) == 1);\n    }\n}\n",
        ),
    )
    .unwrap();
    fs::write(
        prj.root().join("broken.json"),
        candidate(
            "pragma solidity ^0.8.20;\ncontract FeeCheckTest {\n    function unused() public {\n        uint256 x;\n    }\n    function testFee() public {\n        missing();\n    }\n}\n",
        ),
    )
    .unwrap();

    // Forge runs every test in the files, including contracts that no manifest lists.
    fs::write(
        prj.root().join("extra.json"),
        candidate(
            "pragma solidity ^0.8.20;\nimport {Fee} from \"../../contracts/Fee.sol\";\ncontract FeeCheckTest {\n    function testFee() public pure {\n        require(Fee.fee(199) == 1);\n    }\n}\ncontract FeeRoundingTest {\n    function testHiddenRoundsUp() public pure {\n        require(Fee.fee(199) == 2);\n    }\n}\n",
        ),
    )
    .unwrap();
    // A merged invariant campaign reports each predicate on its own.
    fs::write(
        prj.root().join("invariants.json"),
        candidate(
            "pragma solidity ^0.8.20;\nimport {Test} from \"forge-std/Test.sol\";\nimport {Fee} from \"../../contracts/Fee.sol\";\ncontract FeeHandler {\n    uint256 public total;\n    function add(uint256 amount) public {\n        total += Fee.fee(amount % 1e30);\n    }\n}\n/// forge-config: default.invariant.runs = 8\n/// forge-config: default.invariant.depth = 8\ncontract FeeInvariantTest is Test {\n    FeeHandler internal handler;\n    function setUp() public {\n        handler = new FeeHandler();\n        targetContract(address(handler));\n    }\n    function invariant_totalFitsSupply() public view {\n        assertLe(handler.total(), 1e30);\n    }\n    function invariant_totalStaysZero() public view {\n        assertEq(handler.total(), 0);\n    }\n}\n",
        ),
    )
    .unwrap();
    // A handler assertion fails the campaign even when every predicate holds.
    fs::write(
        prj.root().join("handler.json"),
        candidate(
            "pragma solidity ^0.8.20;\nimport {Test} from \"forge-std/Test.sol\";\ncontract BrokenHandler {\n    uint256 public pokes;\n    function poke() public {\n        pokes += 1;\n        assert(false);\n    }\n}\n/// forge-config: default.invariant.runs = 8\n/// forge-config: default.invariant.depth = 8\ncontract HandlerInvariantTest is Test {\n    function setUp() public {\n        targetContract(address(new BrokenHandler()));\n    }\n    function invariant_one() public pure {}\n    function invariant_two() public pure {}\n}\n",
        ),
    )
    .unwrap();
    fs::write(
        prj.root().join("seeded.json"),
        candidate(
            "pragma solidity ^0.8.20;\nimport {Fee} from \"../../contracts/Fee.sol\";\ncontract FeeCheckTest {\n    /// forge-config: default.fuzz.seed = \"0x3\"\n    function testFuzzFee(uint256 amount) public pure {\n        require(Fee.fee(amount) <= amount);\n    }\n}\n",
        ),
    )
    .unwrap();

    cmd.args(["properties", "--check", "passing.json", "--seed", "1", "--seed", "2"])
        .assert_success()
        .stdout_eq(str![[r#"
{
  "passed": true,
  "reasons": [],
  "possible_bugs": []
}

"#]]);
    cmd.forge_fuse()
        .args(["properties", "--check", "broken.json", "--seed", "1", "--seed", "2"])
        .assert_failure()
        .stdout_eq(str![[r#"
{
  "passed": false,
  "reasons": [
    "candidate tests failed on seed 1: Error: Compiler run failed:/nError (7576): Undeclared identifier./n [FILE]:7:9:/n  |/n7 |         missing();/n  |         ^^^^^^^",
    "candidate tests failed on seed 2: Error: Compiler run failed:/nError (7576): Undeclared identifier./n [FILE]:7:9:/n  |/n7 |         missing();/n  |         ^^^^^^^"
  ],
  "possible_bugs": []
}

"#]])
        .stderr_eq(str![[r#"
Error: candidate check failed

"#]]);
    cmd.forge_fuse()
        .args(["properties", "--check", "extra.json", "--seed", "1", "--seed", "2"])
        .assert_failure()
        .stdout_eq(str![[r#"
{
  "passed": false,
  "reasons": [
    "FeeRoundingTest::testHiddenRoundsUp failed on seed 1: EvmError: Revert",
    "FeeRoundingTest::testHiddenRoundsUp failed on seed 2: EvmError: Revert"
  ],
  "possible_bugs": [
    "FeeRoundingTest::testHiddenRoundsUp (seed 1: EvmError: Revert; seed 2: EvmError: Revert)"
  ]
}

"#]]);
    cmd.forge_fuse()
        .args(["properties", "--check", "invariants.json", "--seed", "1", "--seed", "2"])
        .assert_failure()
        .stdout_eq(str![[r#"
{
  "passed": false,
  "reasons": [
    "FeeInvariantTest::invariant_totalStaysZero failed on seed 1: assertion failed: 1 != 0; counterexample: [..]",
    "FeeInvariantTest::invariant_totalStaysZero failed on seed 2: assertion failed: 3 != 0; counterexample: [..]"
  ],
  "possible_bugs": [
    "FeeInvariantTest::invariant_totalStaysZero (seed 1: assertion failed: 1 != 0; counterexample: [..]; seed 2: assertion failed: 3 != 0; counterexample: [..])"
  ]
}

"#]]);
    cmd.forge_fuse()
        .args(["properties", "--check", "handler.json", "--seed", "1", "--seed", "2"])
        .assert_failure()
        .stdout_eq(str![[r#"
{
  "passed": false,
  "reasons": [
    "HandlerInvariantTest::invariant_one failed on seed 1: handler test/generated/FeeCheck.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..]",
    "HandlerInvariantTest::invariant_one failed on seed 2: handler test/generated/FeeCheck.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..]"
  ],
  "possible_bugs": [
    "HandlerInvariantTest::invariant_one (seed 1: handler test/generated/FeeCheck.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..]; seed 2: handler test/generated/FeeCheck.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..])"
  ]
}

"#]]);
    cmd.forge_fuse()
        .args(["properties", "--check", "seeded.json", "--seed", "1", "--seed", "2"])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: candidate test/generated/FeeCheck.t.sol sets a seed in inline config

"#]]);
    assert!(!prj.root().join("test/generated/FeeCheck.t.sol").exists());
}
