//! Tests for the `forge properties` command. Agents are shell scripts, so the tests run only on
//! Unix.
#![cfg(unix)]

use foundry_compilers::artifacts::remappings::Remapping;
use foundry_test_utils::{TestProject, assert_data_eq, forgetest_init, str};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

/// Starts an agent script: `out` is the output directory and `agent` the agent's number.
const PRELUDE: &str = r#"#!/bin/sh
set -eu
out="$FORGE_PROPERTIES_OUT"
agent="${out##*-}"
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

/// A passing test of `Fee`, as an agent writes it.
const FEE_SPEC: &str = r#"cat > "$out/FeeSpec.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Fee} from "../../../src/Fee.sol";
contract FeeSpecTest {
    function testSmallFee() public pure {
        require(Fee.fee(100) == 1);
    }
}
SOL
"#;

/// Writes the agent script and returns the common arguments.
fn properties_args(prj: &TestProject, script: &str) -> Vec<String> {
    let agent = prj.root().join("agent.sh");
    fs::write(&agent, format!("{PRELUDE}{script}")).unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).unwrap();
    [
        "properties",
        "--root",
        prj.root().to_str().unwrap(),
        "--agent-command",
        agent.to_str().unwrap(),
        "--seed",
        "0x5eed",
        "--seed",
        "0xc0ffee",
    ]
    .map(String::from)
    .to_vec()
}

/// Writes the verifier script and returns the arguments that select it. The script starts in the
/// verifier's workspace and appends a line to `cache/verifier-runs` for each run. Forge does not
/// watch the cache for changes.
fn verifier_args(prj: &TestProject, script: &str) -> Vec<String> {
    let verifier = prj.root().join("verifier.sh");
    let runs = prj.root().join("cache/verifier-runs");
    let prelude =
        format!("#!/bin/sh\nset -eu\n[ \"$1\" = --stub ]\necho run >> {}\n", runs.display());
    fs::write(&verifier, format!("{prelude}{script}")).unwrap();
    fs::set_permissions(&verifier, fs::Permissions::from_mode(0o755)).unwrap();
    ["--verifier-command", verifier.to_str().unwrap(), "--verifier-arg=--stub"]
        .map(String::from)
        .to_vec()
}

/// Returns `cache/properties/report.json`.
fn report(prj: &TestProject) -> serde_json::Value {
    let report = fs::read_to_string(prj.root().join("cache/properties/report.json")).unwrap();
    serde_json::from_str(&report).unwrap()
}

/// Returns the paths and reasons of the rejected files in the report, one line each.
fn rejected(prj: &TestProject) -> String {
    report(prj)["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rejected| format!("{} {}", rejected["paths"], rejected["reason"]))
        .collect::<Vec<_>>()
        .join("\n")
}

// Three agents run in parallel, each in one session in its own copy of the project. Forge reports
// a possible bug only when two agents agree on its root cause, keeps the other failures in the
// report file, keeps one copy of identical passing files, and sums the usage that agents report.
#[forgetest_init]
fn properties_reports_bugs_that_agents_agree_on(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    fs::write(prj.root().join("README.md"), "readme-marker").unwrap();
    for path in
        ["CLAUDE.md", "AGENTS.md", "src/CLAUDE.md", ".claude/settings.json", "src/.cursor/rules"]
    {
        let path = prj.root().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "agent-config").unwrap();
    }
    // Generated tests of an earlier run stay out of the copies: this one does not compile.
    fs::create_dir_all(prj.root().join("test/generated")).unwrap();
    fs::write(prj.root().join("test/generated/Old.t.sol"), "contract Old {").unwrap();
    let context = prj.root().join("context.md");
    fs::write(&context, "context-marker: rounding notes").unwrap();
    let mut args = properties_args(
        &prj,
        &format!(
            r#"
prompt="$FORGE_PROPERTIES_PROMPT"
# The prompt names the targets, the context files, and the output directory.
grep -qx 'Find bugs in these targets of the Foundry project in the current directory:' "$prompt"
grep -qx -- '- src/Fee.sol' "$prompt"
grep -qx -- "- $FORGE_PROPERTIES_CONTEXT" "$prompt"
grep -qF "test in $out that fails because of the bug" "$prompt"
printf '%s\n' "$FORGE_PROPERTIES_TARGETS" | grep -qx src/Fee.sol
grep -q context-marker "$FORGE_PROPERTIES_CONTEXT"
case "$FORGE_PROPERTIES_SEED" in 0x*) ;; *) exit 1 ;; esac
# The output directory is in the agent's copy of the project and starts empty.
[ "$(cd "$out" && pwd -P)" = "$(pwd -P)/test/generated/run-1-agent-$agent" ]
[ "$(ls -A test/generated)" = "run-1-agent-$agent" ]
[ -z "$(ls -A "$out")" ]
# The copy has the project's other files, without agent settings and instructions.
grep -q readme-marker README.md
for path in CLAUDE.md AGENTS.md src/CLAUDE.md .claude src/.cursor; do
    [ ! -e "$path" ]
done
{FEE_SPEC}
printf '{{"input_tokens": 1000, "output_tokens": 200, "cost_usd": 0.01}}' > "$FORGE_PROPERTIES_USAGE"
case "$agent" in
1) cat > "$out/FeeCap.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {{Fee}} from "../../../src/Fee.sol";
contract FeeCapTest {{
    // The fee is capped at 100 ether; src/Fee.sol:7 fee() caps the fee at 10 ether.
    function testDocumentedCap() public pure {{
        require(Fee.fee(1e30) == 100 ether);
    }}
}}
SOL
;;
2) cat > "$out/FeeRounding.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {{Fee}} from "../../../src/Fee.sol";
contract FeeRoundingTest {{
    function testRoundsDown() public pure {{
        require(Fee.fee(199) == 1);
    }}
}}
SOL
cat > "$out/FeeLimit.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {{Fee}} from "../../../src/Fee.sol";
contract FeeLimitTest {{
    /// fee(amount) reaches the documented cap for large amounts (src/Fee.sol:7).
    function testReachesCap() public pure {{
        require(Fee.fee(2e22) == 100 ether);
    }}
}}
SOL
;;
3) cat > "$out/FeeCharges.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {{Fee}} from "../../../src/Fee.sol";
contract FeeChargesTest {{
    // A zero amount is charged; src/Fee.sol:11 charges() returns false below 100.
    function testChargesZero() public pure {{
        require(Fee.charges(0));
    }}
}}
SOL
;;
esac
"#
        ),
    );
    args.extend(["--context", context.to_str().unwrap(), "--agents", "3"].map(String::from));
    cmd.args(args).assert_success().stdout_eq(str![[r#"
possible bug (2/3 agents): The fee is capped at 100 ether; src/Fee.sol:7 fee() caps the fee at 10 ether.
  agent 1: FeeCapTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: The fee is capped at 100 ether; src/Fee.sol:7 fee() caps the fee at 10 ether.
    file: cache/properties/run-1/agent-1/tests/FeeCap.t.sol
  agent 2: FeeLimitTest::testReachesCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: fee(amount) reaches the documented cap for large amounts (src/Fee.sol:7).
    file: cache/properties/run-1/agent-2/tests/FeeLimit.t.sol
1 possible bug(s) below the quorum of 2:
single-agent (1/3 agents): A zero amount is charged; src/Fee.sol:11 charges() returns false below 100.
  agent 3: FeeChargesTest::testChargesZero (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: A zero amount is charged; src/Fee.sol:11 charges() returns false below 100.
    file: cache/properties/run-1/agent-3/tests/FeeCharges.t.sol
agent usage: 3000 input tokens, 600 output tokens, 0.03 USD
added 2 generated test file(s):
  test/generated/run-1-agent-1/FeeSpec.t.sol
  test/generated/run-1-agent-2/FeeRounding.t.sol
skipped 2 generated test file(s) that repeat a file of another agent or the project

"#]]);

    let report = report(&prj);
    let below = &report["below_quorum"][0];
    assert_data_eq!(
        format!("{} {}", below["agents"], below["failures"][0]["test"]),
        str![[r#"[3] "FeeChargesTest::testChargesZero""#]]
    );
    assert_data_eq!(
        report["duplicates"].to_string(),
        str![[
            r#"["test/generated/run-1-agent-2/FeeSpec.t.sol","test/generated/run-1-agent-3/FeeSpec.t.sol"]"#
        ]]
    );
    // Forge keeps a copy of each test that an agent wrote, also of the failing ones.
    assert!(prj.root().join("cache/properties/run-1/agent-3/tests/FeeCharges.t.sol").exists());
    assert!(prj.root().join("cache/properties/run-1/agent-1/prompt.md").exists());
}

// A kept test file keeps the file that it imports, while an unrelated file is dropped.
#[forgetest_init]
fn properties_keeps_imported_helpers(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Helper.sol" <<'SOL'
pragma solidity ^0.8.20;
library Helper { function fee(uint256 amount) internal pure returns (uint256) { return amount / 100; } }
SOL
cat > "$out/Property.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Fee} from "../../../src/Fee.sol";
import {Helper} from "./Helper.sol";
contract PropertyTest { function testFee() public pure { require(Fee.fee(199) == Helper.fee(199)); } }
SOL
cat > "$out/Unused.sol" <<'SOL'
pragma solidity ^0.8.20;
library Unused {}
SOL
"#,
    );
    cmd.args(&args).args(["--agents", "1", "--target", "src/Fee.sol"]).assert_success().stdout_eq(
        str![[r#"
added 2 generated test file(s):
  test/generated/run-1-agent-1/Helper.sol
  test/generated/run-1-agent-1/Property.t.sol

"#]],
    );

    assert!(!prj.root().join("test/generated/run-1-agent-1/Unused.sol").exists());
    cmd.forge_fuse().args(["test", "--match-contract", "PropertyTest"]).assert_success();

    // A later run writes to directories of its own. A test that repeats one of the project is
    // written again when a helper that it imports changed, so it runs against that helper.
    let agent = prj.root().join("agent.sh");
    let script = fs::read_to_string(&agent).unwrap().replace("amount / 100", "amount * 1 / 100");
    fs::write(&agent, script).unwrap();
    cmd.forge_fuse().args(&args).args(["--agents", "1", "--target", "src/Fee.sol"]);
    cmd.assert_success().stdout_eq(str![[r#"
added 2 generated test file(s):
  test/generated/run-2-agent-1/Helper.sol
  test/generated/run-2-agent-1/Property.t.sol

"#]]);
    // A run whose files all repeat earlier ones writes nothing, and removes the directory it
    // reserved.
    cmd.forge_fuse().args(&args).args(["--agents", "1", "--target", "src/Fee.sol"]);
    cmd.assert_success().stdout_eq(str![[r#"
no generated test file was kept

"#]]);
    assert!(!prj.root().join("test/generated/run-3-agent-1").exists());
}

// Generated tests run outside the agent's sandbox, so `vm.ffi` fails in Forge's copies of the
// project unless the user passes `--allow-ffi`, and a generated test cannot turn on `ffi` or widen
// `fs_permissions` in inline config, in any form that Foundry reads.
#[forgetest_init]
fn properties_turns_off_ffi_unless_allowed(prj: _, cmd: _) {
    prj.update_config(|config| config.ffi = true);
    // Outside the project: a project file that changes during the run fails it.
    let marker_dir = tempfile::tempdir().unwrap();
    let marker = marker_dir.path().join("ffi-ran");
    let args = properties_args(
        &prj,
        &format!(
            r#"
cat > "$out/Ffi.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {{Test}} from "forge-std/Test.sol";
contract FfiTest is Test {{
    function testFfi() public {{
        string[] memory command = new string[](2);
        command[0] = "touch";
        command[1] = "{}";
        vm.ffi(command);
    }}
}}
SOL
cat > "$out/InlineFfi.t.sol" <<'SOL'
pragma solidity ^0.8.20;
contract InlineFfiTest {{
    /// forge-config: default.ffi = true
    function testFfi() public pure {{}}
}}
SOL
cat > "$out/InlineFs.t.sol" <<'SOL'
pragma solidity ^0.8.20;
contract InlineFsTest {{
    /// forge-config: default.fs-permissions = [{{ access = "read-write", path = "/" }}]
    function testWrite() public pure {{}}
}}
SOL
"#,
            marker.display()
        ),
    );
    cmd.args(&args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
no generated test file was kept

"#]]).stderr_eq(str![[r#"
Warning: 3 failing test(s) used something that Forge blocks (ffi, environment, network, or inline config), so they are not possible bugs: see cache/properties/report.json

"#]]);
    assert!(!marker.exists());
    let blocked = report(&prj)["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|failure| format!("{} {}", failure["test"], failure["reason"]))
        .collect::<Vec<_>>();
    assert_data_eq!(
        blocked.join("\n"),
        str![[r#"
"FfiTest::testFfi" "seed 24301: vm.ffi: FFI is disabled; add the `--ffi` flag to allow tests to call external commands; seed 12648430: vm.ffi: FFI is disabled; add the `--ffi` flag to allow tests to call external commands"
"InlineFfiTest::testFfi" "seed 24301: inline config cannot change `ffi` in generated tests; seed 12648430: inline config cannot change `ffi` in generated tests"
"InlineFsTest::testWrite" "seed 24301: inline config cannot change `fs_permissions` in generated tests; seed 12648430: inline config cannot change `fs_permissions` in generated tests"
"#]]
    );

    cmd.forge_fuse().args(&args).args(["--agents", "1", "--allow-ffi"]).assert_success();
    assert!(marker.exists());
    assert!(prj.root().join("test/generated/run-1-agent-1/Ffi.t.sol").exists());
}

// Generated tests cannot read Forge's environment or open network connections. Their failures
// are not possible bugs, even when agents agree on them; Forge counts them in one warning.
#[forgetest_init]
fn properties_blocks_environment_and_network(prj: _, cmd: _) {
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Sandbox.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Test} from "forge-std/Test.sol";
contract SandboxTest is Test {
    // The secret is set (src/Secret.sol:1).
    function testSecret() public view {
        vm.envString("SECRET");
    }
    // The fork opens (src/Fork.sol:1).
    function testFork() public {
        vm.createFork("http://127.0.0.1:1");
    }
}
SOL
"#,
    );
    cmd.env("SECRET", "secret-marker");
    cmd.args(args).args(["--agents", "2"]).assert_success().stdout_eq(str![[r#"
no generated test file was kept

"#]]).stderr_eq(str![[r#"
Warning: 4 failing test(s) used something that Forge blocks (ffi, environment, network, or inline config), so they are not possible bugs: see cache/properties/report.json

"#]]);

    let blocked = report(&prj)["blocked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|failure| format!("{} {} {}", failure["agent"], failure["test"], failure["reason"]))
        .collect::<Vec<_>>();
    assert_data_eq!(
        blocked.join("\n"),
        str![[r#"
1 "SandboxTest::testFork" "seed 24301: vm.createFork: network access is disabled; seed 12648430: vm.createFork: network access is disabled"
1 "SandboxTest::testSecret" "seed 24301: vm.envString: environment variable /"SECRET/" not found; seed 12648430: vm.envString: environment variable /"SECRET/" not found"
2 "SandboxTest::testFork" "seed 24301: vm.createFork: network access is disabled; seed 12648430: vm.createFork: network access is disabled"
2 "SandboxTest::testSecret" "seed 24301: vm.envString: environment variable /"SECRET/" not found; seed 12648430: vm.envString: environment variable /"SECRET/" not found"
"#]]
    );
}

// At the timeout, Forge ends the agent's whole process group and runs the tests that it wrote. An
// agent that exits with an error fails the run, but its tests still count.
#[forgetest_init]
fn properties_runs_tests_after_timeout_and_agent_error(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let marker = prj.root().join("still-running");
    let args = properties_args(
        &prj,
        r#"
cat > "$out/FeeCap.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Fee} from "../../../src/Fee.sol";
contract FeeCapTest {
    // The fee is capped at 100 ether; src/Fee.sol:7 fee().
    function testDocumentedCap() public pure {
        require(Fee.fee(1e30) == 100 ether);
    }
}
SOL
if [ "$agent" = 1 ]; then
    (sleep 8; touch "$MARKER") &
    sleep 30
fi
echo "quota exceeded" >&2
exit 3
"#,
    );
    cmd.env("MARKER", &marker);
    let start = Instant::now();
    cmd.args(args).args(["--agents", "2", "--timeout", "5"]).assert_failure().stdout_eq(str![[r#"
possible bug (2/2 agents): The fee is capped at 100 ether; src/Fee.sol:7 fee().
  agent 1: FeeCapTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: The fee is capped at 100 ether; src/Fee.sol:7 fee().
    file: cache/properties/run-1/agent-1/tests/FeeCap.t.sol
  agent 2: FeeCapTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: The fee is capped at 100 ether; src/Fee.sol:7 fee().
    file: cache/properties/run-1/agent-2/tests/FeeCap.t.sol
no generated test file was kept

"#]]).stderr_eq(str![[r#"
Warning: 1 agent(s) reached the timeout, so Forge ended them and ran the tests that they wrote
Error: agent 2: agent command failed (exit status: 3): see cache/properties/run-1/agent-2/agent.log

"#]]);

    assert_eq!(report(&prj)["timed_out"].to_string(), "[1]");
    assert_eq!(
        fs::read_to_string(prj.root().join("cache/properties/run-1/agent-2/agent.log")).unwrap(),
        "quota exceeded\n"
    );
    // The process that the agent started in the background ended with it.
    thread::sleep(Duration::from_secs(9).saturating_sub(start.elapsed()));
    assert!(!marker.exists());
}

// The target is in a linked dependency directory. It imports a library through a remapping and the
// test imports the library by its project path; both must name the same file in the workspace, or
// the types differ.
#[forgetest_init]
fn properties_keeps_library_paths_in_workspaces(prj: _, cmd: _) {
    fs::create_dir_all(prj.root().join("lib/example")).unwrap();
    fs::write(
        prj.root().join("lib/example/Kind.sol"),
        "pragma solidity ^0.8.20;\nenum Kind { Small, Large }\ninterface IBucket {\n    function bucket(uint256 value) external pure returns (uint256);\n}\n",
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
cat > "$out/ArithmeticLower.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Arithmetic} from "bucket/Arithmetic.sol";
contract ArithmeticLowerTest {
    Arithmetic internal arithmetic = new Arithmetic();
    function testBoundary() public view {
        require(arithmetic.bucket(9) == 1);
        require(arithmetic.bucket(10) == 2);
    }
}
SOL
"#,
    );
    cmd.args(args)
        .args(["--agents", "1", "--target", "node_modules/bucket/Arithmetic.sol"])
        .assert_success()
        .stdout_eq(str![[r#"
added 1 generated test file(s):
  test/generated/run-1-agent-1/ArithmeticLower.t.sol

"#]]);
}

// A generated file must not leave the project through a symlinked directory, so the run stops
// before it reserves its output directories.
#[forgetest_init]
fn properties_rejects_symlinked_generated_directory(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), prj.root().join("test/generated")).unwrap();
    let args = properties_args(&prj, FEE_SPEC);
    cmd.args(args).args(["--agents", "1"]).assert_failure().stderr_eq(str![[r#"
Error: generated test path test/generated escapes project root [..]

"#]]);

    assert!(!outside.path().join("run-1-agent-1").exists());
    assert!(!outside.path().join("FeeSpec.t.sol").exists());
}

// Test runs happen in a copy of the project. Inherited config, profiles, `FOUNDRY_CONFIG`, a
// source directory other than `src`, and negative filters must apply as in the project, and
// filters must not hide generated tests. Invariant predicates and handler failures count on their
// own, and a file that does not compile does not hide the other files.
#[forgetest_init]
fn properties_runs_every_generated_test(prj: _, cmd: _) {
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
    let args = properties_args(
        &prj,
        r#"
cat > "$out/FeeCheck.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Fee} from "fee/Fee.sol";
contract FeeCheckTest {
    /// forge-config: ci.fuzz.runs = 10
    function testFee() public pure {
        require(Fee.fee(199) == 1);
    }
}
SOL
[ -n "${ONLY_PASSING:-}" ] && exit 0
cat > "$out/FeeRounding.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Fee} from "../../../contracts/Fee.sol";
contract FeeRoundingTest {
    // The fee rounds up (contracts/Fee.sol:4).
    function testHiddenRoundsUp() public pure {
        require(Fee.fee(199) == 2);
    }
}
SOL
cat > "$out/FeeInvariant.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Test} from "forge-std/Test.sol";
import {Fee} from "../../../contracts/Fee.sol";
contract FeeHandler {
    uint256 public total;
    function add(uint256 amount) public {
        total += Fee.fee(amount % 1e30);
    }
}
/// forge-config: default.invariant.runs = 8
/// forge-config: default.invariant.depth = 8
contract FeeInvariantTest is Test {
    FeeHandler internal handler;
    function setUp() public {
        handler = new FeeHandler();
        targetContract(address(handler));
    }
    function invariant_totalFitsSupply() public view {
        assertLe(handler.total(), 1e30);
    }
    function invariant_totalStaysZero() public view {
        assertEq(handler.total(), 0);
    }
}
SOL
cat > "$out/Handler.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Test} from "forge-std/Test.sol";
contract BrokenHandler {
    uint256 public pokes;
    function poke() public {
        pokes += 1;
        assert(false);
    }
}
/// forge-config: default.invariant.runs = 8
/// forge-config: default.invariant.depth = 8
contract HandlerInvariantTest is Test {
    function setUp() public {
        targetContract(address(new BrokenHandler()));
    }
    function invariant_one() public pure {}
    function invariant_two() public pure {}
}
SOL
cat > "$out/Broken.t.sol" <<'SOL'
pragma solidity ^0.8.20;
contract BrokenTest {
    function testFee() public {
        missing();
    }
}
SOL
cat > "$out/Seeded.t.sol" <<'SOL'
pragma solidity ^0.8.20;
contract SeededTest {
    /** forge-config: default.fuzz = { seed = "0x3", runs = 1 } */
    function testFuzzFee(uint256 amount) public pure {}
}
SOL
"#,
    );
    cmd.args(&args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
possible bug (1/1 agents): The fee rounds up (contracts/Fee.sol:4).
  agent 1: FeeRoundingTest::testHiddenRoundsUp (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: The fee rounds up (contracts/Fee.sol:4).
    file: cache/properties/run-1/agent-1/tests/FeeRounding.t.sol
possible bug (1/1 agents): FeeInvariantTest::invariant_totalStaysZero
  agent 1: FeeInvariantTest::invariant_totalStaysZero (seed 24301: assertion failed: [..] != 0; counterexample: [..]; seed 12648430: assertion failed: [..] != 0; counterexample: [..])
    file: cache/properties/run-1/agent-1/tests/FeeInvariant.t.sol
possible bug (1/1 agents): HandlerInvariantTest::invariant_one
  agent 1: HandlerInvariantTest::invariant_one (seed 24301: handler test/generated/run-1-agent-1/Handler.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..]; seed 12648430: handler test/generated/run-1-agent-1/Handler.t.sol:BrokenHandler::poke: panic: assertion failed (0x01); counterexample: [..])
    file: cache/properties/run-1/agent-1/tests/Handler.t.sol
1 generated test file(s) did not compile or were not allowed: see cache/properties/report.json
added 1 generated test file(s):
  test/generated/run-1-agent-1/FeeCheck.t.sol

"#]]).stderr_eq(str![[r#"
Warning: 1 failing test(s) used something that Forge blocks (ffi, environment, network, or inline config), so they are not possible bugs: see cache/properties/report.json

"#]]);
    assert_data_eq!(
        rejected(&prj),
        str![[
            r#"["test/generated/run-1-agent-1/Broken.t.sol"] "Error: Compiler run failed:/nError (7576): Undeclared identifier./n --> test/generated/run-1-agent-1/Broken.t.sol:4:9:/n  |/n4 |         missing();/n  |         ^^^^^^^""#
        ]]
    );

    // The resolved workspace test path must win over an absolute inherited environment path.
    cmd.forge_fuse();
    cmd.env("ONLY_PASSING", "1");
    cmd.env("FOUNDRY_TEST", prj.root().join("test"));
    cmd.args(&args).args(["--agents", "1"]).assert_success();
    // A config file chosen with `FOUNDRY_CONFIG` is resolved in the project, not in the copy.
    fs::copy(&foundry_toml, prj.root().join("custom.toml")).unwrap();
    cmd.forge_fuse();
    cmd.env("ONLY_PASSING", "1");
    cmd.env("FOUNDRY_CONFIG", "custom.toml");
    cmd.args(&args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
no generated test file was kept

"#]]);
    assert_eq!(
        report(&prj)["duplicates"].to_string(),
        r#"["test/generated/run-2-agent-1/FeeCheck.t.sol"]"#
    );
}

// A remapping to a generated test directory is rebased into the copy, wherever it is set.
#[forgetest_init]
fn properties_rebases_remapping_providers(prj: _, cmd: _) {
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Helper.sol" <<'SOL'
pragma solidity ^0.8.20;
library Helper { function value() internal pure returns (uint) { return 1; } }
SOL
dir=$(basename "$out")
cat > "$out/Property.t.sol" <<SOL
pragma solidity ^0.8.20;
import {Helper} from 'generated/$dir/Helper.sol';
contract Property { function testValue() public pure { require(Helper.value() == 1); } }
SOL
"#,
    );
    let mapping = format!("generated/={}/test/generated/", prj.root().display());
    fs::write(prj.root().join("remappings.txt"), &mapping).unwrap();
    cmd.args(&args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
added 2 generated test file(s):
  test/generated/run-1-agent-1/Helper.sol
  test/generated/run-1-agent-1/Property.t.sol

"#]]);
    fs::remove_file(prj.root().join("remappings.txt")).unwrap();
    for variable in ["FOUNDRY_REMAPPINGS", "DAPP_REMAPPINGS"] {
        cmd.forge_fuse().env(variable, &mapping);
        cmd.args(&args).args(["--agents", "1"]).assert_success();
        assert_eq!(report(&prj)["kept"].as_array().unwrap().len(), 2);
        assert_eq!(report(&prj)["rejected"].to_string(), "[]");
        cmd.unset_env(variable);
    }
}

// A pnpm package is a link into `node_modules/.pnpm`, and it imports its own files by absolute
// paths such as `src/lib/Init.sol`. The workspace must resolve them as the project does.
#[forgetest_init]
fn properties_resolves_pnpm_package_imports(prj: _, cmd: _) {
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
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Stub.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Proxy} from "pkg/utils/Proxy.sol";
contract StubTest {
    function testValue() public {
        require(new Proxy().value() == 7);
    }
}
SOL
"#,
    );
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
added 1 generated test file(s):
  test/generated/run-1-agent-1/Stub.t.sol

"#]]);
}

// A project test imports a file from a directory that is not a source, test, script, or library
// directory. The workspace must still have it.
#[forgetest_init]
fn properties_copies_other_project_directories(prj: _, cmd: _) {
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
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Stub.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Helper} from "../../utils/Helper.sol";
contract StubTest {
    function testChainId() public pure {
        require(Helper.chainId() == 2017);
    }
}
SOL
"#,
    );
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
added 1 generated test file(s):
  test/generated/run-1-agent-1/Stub.t.sol

"#]]);
}

// With `deny = "warnings"`, the project's ignored warning paths must still apply in the workspace,
// and a warning in a generated test is a compiler error that rejects only its file.
#[forgetest_init]
fn properties_rejects_denied_warnings(prj: _, cmd: _) {
    prj.add_test(
        "legacy/Legacy.t.sol",
        "pragma solidity ^0.8.20;\ncontract LegacyTest {\n    function testLegacy() public pure {\n        uint256 unused = 1;\n    }\n}\n",
    );
    prj.update_config(|config| {
        config.deny = foundry_config::DenyLevel::Warnings;
        config.ignored_file_paths = vec!["test/legacy".into()];
    });
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Passing.t.sol" <<'SOL'
pragma solidity ^0.8.20;
contract PassingTest {
    function testStub() public pure {}
}
SOL
cat > "$out/Warning.t.sol" <<'SOL'
pragma solidity ^0.8.20;
contract WarningTest {
    function testStub() public pure {
        uint256 unused = 1;
    }
}
SOL
"#,
    );
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
1 generated test file(s) did not compile or were not allowed: see cache/properties/report.json
added 1 generated test file(s):
  test/generated/run-1-agent-1/Passing.t.sol

"#]]);
    assert_data_eq!(
        rejected(&prj),
        str![[r#"
["test/generated/run-1-agent-1/Warning.t.sol"] "the project denies compiler warnings: Warning (2072): Unused local variable./n --> test/generated/run-1-agent-1/Warning.t.sol:4:9:/n  |/n4 |         uint256 unused = 1;/n  |         ^^^^^^^^^^^^^^"
"#]]
    );
}

// Forge builds the project once, and every workspace starts from that build output, so a build in
// a workspace compiles only the files that it adds. This holds also when the project denies
// warnings, which turns Foundry's compiler cache off.
#[forgetest_init]
fn properties_workspaces_reuse_the_project_build(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    prj.update_config(|config| config.deny = foundry_config::DenyLevel::Warnings);
    let mut args = properties_args(
        &prj,
        &format!(
            r#"
forge="$1"
"$forge" build > build.log 2>&1
grep -q 'No files changed, compilation skipped' build.log
{FEE_SPEC}
"$forge" test --match-path "test/generated/run-1-agent-$agent/FeeSpec.t.sol" > test.log 2>&1
# The compiler builds only the new test, not the project's other files.
grep -q 'Compiling 1 files' test.log
"#
        ),
    );
    args.extend(["--agent-arg".to_string(), prj.foundry_bin_path("forge").display().to_string()]);
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
added 1 generated test file(s):
  test/generated/run-1-agent-1/FeeSpec.t.sol

"#]]);
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

    function withdraw(uint256 amount) external {
        balanceOf[msg.sender] -= amount;
    }
}
"#;

// By default four agents run with a quorum of two.
#[forgetest_init]
fn properties_runs_four_agents_by_default(prj: _, cmd: _) {
    prj.add_source("Vault.sol", VAULT);
    let args = properties_args(
        &prj,
        r#"
head='pragma solidity ^0.8.20;
import {Test} from "forge-std/Test.sol";
import {Vault} from "../../../src/Vault.sol";'
setup='    Vault internal vault;
    function setUp() public {
        vault = new Vault();
    }'
zero='    // deposit(amount) accepts a zero amount; src/Vault.sol:11 deposit() reverts ZeroAmount.
    function testDepositZero() public {
        vault.deposit(0);
    }'
shares='    // shares(assets) returns one share per asset; src/Vault.sol:17 shares() halves the assets.
    function testShares() public view {
        assertEq(vault.shares(10), 10);
    }'
case "$agent" in
1) printf '%s\ncontract VaultBugTest is Test {\n%s\n%s\n%s\n}\n' "$head" "$setup" "$zero" "$shares" > "$out/VaultBug.t.sol" ;;
2) printf '%s\ncontract VaultBugTest is Test {\n%s\n%s\n}\n' "$head" "$setup" "$shares" > "$out/VaultBug.t.sol" ;;
*) printf '%s\ncontract VaultSpecTest is Test {\n%s\n    function testDepositFive() public {\n        vault.deposit(5);\n    }\n}\n' "$head" "$setup" > "$out/VaultSpec.t.sol" ;;
esac
"#,
    );
    cmd.args(args).args(["--target", "src/Vault.sol"]).assert_success().stdout_eq(str![[r#"
possible bug (2/4 agents): shares(assets) returns one share per asset; src/Vault.sol:17 shares() halves the assets.
  agent 1: VaultBugTest::testShares (seed 24301: assertion failed: 5 != 10; seed 12648430: assertion failed: 5 != 10)
    claim: shares(assets) returns one share per asset; src/Vault.sol:17 shares() halves the assets.
    file: cache/properties/run-1/agent-1/tests/VaultBug.t.sol
  agent 2: VaultBugTest::testShares (seed 24301: assertion failed: 5 != 10; seed 12648430: assertion failed: 5 != 10)
    claim: shares(assets) returns one share per asset; src/Vault.sol:17 shares() halves the assets.
    file: cache/properties/run-1/agent-2/tests/VaultBug.t.sol
1 possible bug(s) below the quorum of 2:
single-agent (1/4 agents): deposit(amount) accepts a zero amount; src/Vault.sol:11 deposit() reverts ZeroAmount.
  agent 1: VaultBugTest::testDepositZero (seed 24301: ZeroAmount(); seed 12648430: ZeroAmount())
    claim: deposit(amount) accepts a zero amount; src/Vault.sol:11 deposit() reverts ZeroAmount.
    file: cache/properties/run-1/agent-1/tests/VaultBug.t.sol
added 1 generated test file(s):
  test/generated/run-1-agent-3/VaultSpec.t.sol
skipped 1 generated test file(s) that repeat a file of another agent or the project

"#]]);
}

// Agents work in copies whose dependency entries link to the project, so a project file that
// changes while the agents run, also behind a linked package as pnpm makes them, fails the run and
// Forge keeps no test. A failing test without a comment has its name as the claim.
#[forgetest_init]
fn properties_reports_changed_files(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let store = tempfile::tempdir().unwrap();
    fs::write(store.path().join("notes.txt"), "original").unwrap();
    fs::create_dir_all(prj.root().join("node_modules")).unwrap();
    std::os::unix::fs::symlink(store.path(), prj.root().join("node_modules/pkg")).unwrap();
    let args = properties_args(
        &prj,
        &format!(
            r#"
echo changed > node_modules/pkg/notes.txt
{FEE_SPEC}
cat > "$out/FeeCap.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {{Fee}} from "../../../src/Fee.sol";
contract FeeCapTest {{
    function testDocumentedCap() public pure {{
        require(Fee.fee(1e30) == 100 ether);
    }}
}}
SOL
"#
        ),
    );
    cmd.args(args).args(["--agents", "1", "--target", "src/Fee.sol"]).assert_failure().stdout_eq(
        str![[r#"
possible bug (1/1 agents): FeeCapTest::testDocumentedCap
  agent 1: FeeCapTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    file: cache/properties/run-1/agent-1/tests/FeeCap.t.sol

"#]],
    )
    .stderr_eq(str![[r#"
Error: 1 project file(s) changed while the agents ran, so Forge kept no generated test: see cache/properties/report.json

"#]]);

    let report = report(&prj);
    assert_eq!(report["tainted"].to_string(), r#"["node_modules/pkg/notes.txt"]"#);
    assert_eq!(report["kept"].to_string(), "[]");
    assert!(!prj.root().join("test/generated/run-1-agent-1/FeeSpec.t.sol").exists());
}

// A Soldeer project has `soldeer.lock` and its dependencies under `dependencies/`, which the
// workspace links. `forge build` checks the lock by hashing each dependency, and the hash panics on
// a link, so the workspace drops the lock and the agent's tests still run.
#[forgetest_init]
fn properties_runs_tests_of_soldeer_projects(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    fs::create_dir_all(prj.root().join("dependencies/dep-1.0.0/src")).unwrap();
    fs::write(
        prj.root().join("dependencies/dep-1.0.0/src/Dep.sol"),
        "pragma solidity ^0.8.20;\nlibrary Dep {}\n",
    )
    .unwrap();
    fs::write(
        prj.root().join("soldeer.lock"),
        r#"[[dependencies]]
name = "dep"
version = "1.0.0"
url = "https://soldeer-revisions.s3.amazonaws.com/dep/1_0_0.zip"
checksum = "0000000000000000000000000000000000000000000000000000000000000000"
integrity = "0000000000000000000000000000000000000000000000000000000000000000"
"#,
    )
    .unwrap();
    prj.update_config(|config| config.libs.push("dependencies".into()));
    let args = properties_args(
        &prj,
        r#"
[ ! -e soldeer.lock ]
cat > "$out/FeeCap.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Fee} from "../../../src/Fee.sol";
contract FeeCapTest {
    function testDocumentedCap() public pure {
        require(Fee.fee(1e30) == 100 ether);
    }
}
SOL
"#,
    );
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
possible bug (1/1 agents): FeeCapTest::testDocumentedCap
  agent 1: FeeCapTest::testDocumentedCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    file: cache/properties/run-1/agent-1/tests/FeeCap.t.sol
no generated test file was kept

"#]]);
}

// A project test imports a nested library with a relative path that climbs above the root, so the
// compiler reads the file itself, through the workspace's link to the library. The compiler
// resolves the link before it checks the allowed paths, so the project's library directory must
// be allowed.
#[forgetest_init]
fn properties_allows_nested_library_links(prj: _, cmd: _) {
    fs::create_dir_all(prj.root().join("lib/common/lib/nested")).unwrap();
    fs::write(
        prj.root().join("lib/common/lib/nested/Nested.sol"),
        "pragma solidity ^0.8.20;\nlibrary Nested {\n    function value() internal pure returns (uint256) {\n        return 7;\n    }\n}\n",
    )
    .unwrap();
    prj.add_test(
        "unit/Nested.t.sol",
        "pragma solidity ^0.8.20;\nimport {Nested} from \"../../../lib/common/lib/nested/Nested.sol\";\ncontract NestedTest {\n    function testValue() public pure {\n        require(Nested.value() == 7);\n    }\n}\n",
    );
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Stub.t.sol" <<'SOL'
pragma solidity ^0.8.20;
contract StubTest {
    function testTrue() public pure {}
}
SOL
"#,
    );
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
added 1 generated test file(s):
  test/generated/run-1-agent-1/Stub.t.sol

"#]]);
}

// The prompt asks for a `@bug` tag, and Forge groups tagged tests on the target file and function,
// whatever their words. Groups below the quorum follow the possible bugs as a single-agent list.
#[forgetest_init]
fn properties_groups_tagged_claims(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let args = properties_args(
        &prj,
        r#"
grep -qxF '// @bug <Contract>.<function> <file>:<line> — <claim>' "$FORGE_PROPERTIES_PROMPT"
case "$agent" in
1) claim='Fee.fee src/Fee.sol:7 — the fee is capped at 10 ether, not 100 ether.' ;;
2) claim='Fee.fee src/Fee.sol:8 — large amounts pay too little.' ;;
*) claim='Fee.charges src/Fee.sol:11 — a zero amount is charged.' ;;
esac
cat > "$out/FeeBug.t.sol" <<SOL
pragma solidity ^0.8.20;
import {Fee} from "../../../src/Fee.sol";
contract FeeBugTest {
    // @bug $claim
    function testBug() public pure {
        require(Fee.fee(1e30) == 100 ether);
    }
}
SOL
"#,
    );
    cmd.args(args).args(["--agents", "3"]).assert_success().stdout_eq(str![[r#"
possible bug (2/3 agents): @bug Fee.fee src/Fee.sol:7 — the fee is capped at 10 ether, not 100 ether.
  agent 1: FeeBugTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.fee src/Fee.sol:7 — the fee is capped at 10 ether, not 100 ether.
    file: cache/properties/run-1/agent-1/tests/FeeBug.t.sol
  agent 2: FeeBugTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.fee src/Fee.sol:8 — large amounts pay too little.
    file: cache/properties/run-1/agent-2/tests/FeeBug.t.sol
1 possible bug(s) below the quorum of 2:
single-agent (1/3 agents): @bug Fee.charges src/Fee.sol:11 — a zero amount is charged.
  agent 3: FeeBugTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.charges src/Fee.sol:11 — a zero amount is charged.
    file: cache/properties/run-1/agent-3/tests/FeeBug.t.sol
no generated test file was kept

"#]]);
}

// The workspace config keeps the project's other profiles. Names that are not bare TOML keys, such
// as an empty name, a name with a dot, or a name with a space, stay quoted. Internal runs keep
// their own `--rerun` state; the project's file stays as it is.
#[forgetest_init]
fn properties_keeps_quoted_profile_names(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let failures = prj.root().join("cache/test-failures");
    fs::create_dir_all(failures.parent().unwrap()).unwrap();
    fs::write(&failures, r#"{"version":1,"failures":[]}"#).unwrap();
    let foundry_toml = prj.root().join("foundry.toml");
    let toml = fs::read_to_string(&foundry_toml).unwrap();
    fs::write(
        &foundry_toml,
        toml + "\n[profile.\"\"]\n\n[profile.\"ci.fast\"]\n\n[profile.\"with space\"]\n",
    )
    .unwrap();
    let args = properties_args(&prj, FEE_SPEC);
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
added 1 generated test file(s):
  test/generated/run-1-agent-1/FeeSpec.t.sol

"#]]);
    assert_eq!(fs::read_to_string(&failures).unwrap(), r#"{"version":1,"failures":[]}"#);
}

/// Two agents write a test for each name in `names`, and a third writes a test for another bug. The
/// tests fail, and every name is a bug group with two agents. The third agent's group is below the
/// quorum.
fn bug_agents(prj: &TestProject, names: &[&str]) -> Vec<String> {
    properties_args(
        prj,
        &format!(
            r#"
for name in {}; do
    lower=$(printf '%s' "$name" | tr 'A-Z' 'a-z')
    if [ "$agent" = 3 ]; then lower=single; name=Single; fi
    cat > "$out/$name.t.sol" <<SOL
pragma solidity ^0.8.20;
contract ${{name}}Test {{
    // @bug Fee.$lower src/Fee.sol:7 — the $lower is wrong.
    function testBug() public pure {{
        require(false);
    }}
}}
SOL
done
"#,
            names.join(" ")
        ),
    )
}

// A verifier runs once per group that reaches the quorum, in a fresh workspace that holds only the
// tests of the group. Forge lists confirmed groups first, then groups that need validation with the
// open fact, then rejected groups with the disproof, and records each verdict. A group without a
// verdict needs validation. Groups below the quorum are not verified.
#[forgetest_init]
fn properties_verifies_groups_that_reach_the_quorum(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    for path in ["CLAUDE.md", ".env"] {
        fs::write(prj.root().join(path), "secret-marker").unwrap();
    }
    let mut args = bug_agents(&prj, &["Confirmed", "Missing", "Rejected", "Unresolved"]);
    args.extend(verifier_args(
        &prj,
        r#"
# The prompt, the group file, and the verdict file are named in the task.
grep -q 'Your job is to DISPROVE it' "$FORGE_PROPERTIES_VERIFY_PROMPT"
grep -qF -- "$FORGE_PROPERTIES_GROUP" "$FORGE_PROPERTIES_VERIFY_PROMPT"
grep -qF -- "$FORGE_PROPERTIES_VERDICT" "$FORGE_PROPERTIES_VERIFY_PROMPT"
grep -q '"claim"' "$FORGE_PROPERTIES_GROUP"
# The workspace holds the tests of this group only, and no agent settings or hidden files.
[ "$(ls test/generated)" = "run-1-agent-1
run-1-agent-2" ]
[ "$(ls test/generated/run-1-agent-1 | wc -l)" -eq 1 ]
[ "$(ls test/generated/run-1-agent-2 | wc -l)" -eq 1 ]
[ ! -e CLAUDE.md ] && [ ! -e .env ]
name=$(ls test/generated/run-1-agent-1)
grep -qF -- "- file: test/generated/run-1-agent-1/$name" "$FORGE_PROPERTIES_VERIFY_PROMPT"
case "$name" in
Confirmed.t.sol)
    cat > "$FORGE_PROPERTIES_VERDICT" <<'JSON'
{"verdict": "confirmed", "reason": "Every disproof failed.", "evidence": [{"file": "src/Fee.sol", "line": 7, "quote": "f = amount / 100;"}], "unresolved": ""}
JSON
    ;;
Unresolved.t.sol)
    cat > "$FORGE_PROPERTIES_VERDICT" <<'JSON'
{"verdict": "needs_validation", "reason": "The docs are silent.", "unresolved": "Whether the docs promise the floor."}
JSON
    ;;
Rejected.t.sol)
    cat > "$FORGE_PROPERTIES_VERDICT" <<'JSON'
{"verdict": "rejected", "reason": "The cap is documented.", "evidence": [{"file": "src/Fee.sol", "line": 25, "quote": "Charges 1% of `amount`,\n  capped at 100 ether."}]}
JSON
    ;;
esac
printf '{"input_tokens": 500, "output_tokens": 100, "cost_usd": 0.02}' > "$FORGE_PROPERTIES_USAGE"
"#,
    ));
    cmd.args(args).args(["--agents", "3"]).assert_success().stdout_eq(str![[r#"
confirmed (2/3 agents): @bug Fee.confirmed src/Fee.sol:7 — the confirmed is wrong.
  agent 1: ConfirmedTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.confirmed src/Fee.sol:7 — the confirmed is wrong.
    file: cache/properties/run-1/agent-1/tests/Confirmed.t.sol
  agent 2: ConfirmedTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.confirmed src/Fee.sol:7 — the confirmed is wrong.
    file: cache/properties/run-1/agent-2/tests/Confirmed.t.sol
  verifier: Every disproof failed.
    evidence: src/Fee.sol:7: f = amount / 100;
needs validation (2/3 agents): @bug Fee.missing src/Fee.sol:7 — the missing is wrong.
  agent 1: MissingTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.missing src/Fee.sol:7 — the missing is wrong.
    file: cache/properties/run-1/agent-1/tests/Missing.t.sol
  agent 2: MissingTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.missing src/Fee.sol:7 — the missing is wrong.
    file: cache/properties/run-1/agent-2/tests/Missing.t.sol
  verifier error: the verifier wrote no verdict: see cache/properties/run-1/verify-2/verifier.log
needs validation (2/3 agents): @bug Fee.unresolved src/Fee.sol:7 — the unresolved is wrong.
  agent 1: UnresolvedTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.unresolved src/Fee.sol:7 — the unresolved is wrong.
    file: cache/properties/run-1/agent-1/tests/Unresolved.t.sol
  agent 2: UnresolvedTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.unresolved src/Fee.sol:7 — the unresolved is wrong.
    file: cache/properties/run-1/agent-2/tests/Unresolved.t.sol
  verifier: The docs are silent.
    unresolved: Whether the docs promise the floor.
rejected (2/3 agents): @bug Fee.rejected src/Fee.sol:7 — the rejected is wrong.
  agent 1: RejectedTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.rejected src/Fee.sol:7 — the rejected is wrong.
    file: cache/properties/run-1/agent-1/tests/Rejected.t.sol
  agent 2: RejectedTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.rejected src/Fee.sol:7 — the rejected is wrong.
    file: cache/properties/run-1/agent-2/tests/Rejected.t.sol
  verifier: The cap is documented.
    evidence: src/Fee.sol:25: Charges 1% of `amount`, capped at 100 ether.
1 possible bug(s) below the quorum of 2:
single-agent (1/3 agents): @bug Fee.single src/Fee.sol:7 — the single is wrong.
  agent 3: SingleTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.single src/Fee.sol:7 — the single is wrong.
    file: cache/properties/run-1/agent-3/tests/Single.t.sol
verifier usage: 2000 input tokens, 400 output tokens, 0.08 USD
no generated test file was kept

"#]]);

    let report = report(&prj);
    assert_eq!(
        report["possible_bugs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| group["verification"]["verdict"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["confirmed", "needs_validation", "needs_validation", "rejected"]
    );
    assert!(report["below_quorum"][0].get("verification").is_none());
    assert_data_eq!(
        report["possible_bugs"][0]["verification"].to_string(),
        str![[
            r#"{"verdict":"confirmed","reason":"Every disproof failed.","evidence":[{"file":"src/Fee.sol","line":7,"quote":"f = amount / 100;"}],"unresolved":"","usage":{"input_tokens":500,"output_tokens":100,"cost_usd":0.02}}"#
        ]]
    );
    // One run for each group above the quorum.
    let runs = fs::read_to_string(prj.root().join("cache/verifier-runs")).unwrap();
    assert_eq!(runs.lines().count(), 4);
    assert!(prj.root().join("cache/properties/run-1/verify-1/prompt.md").exists());
}

// A verifier that leaves no usable verdict leaves its group in need of validation, with the reason.
// An invalid verdict, a rejection without evidence, a timeout, and a failed exit all count. Forge
// still reads a valid verdict that a timed-out or failed verifier wrote.
#[forgetest_init]
fn properties_verifier_errors_need_validation(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let mut args = bug_agents(&prj, &["Failing", "Invalid", "NoEvidence", "Slow"]);
    args.extend(verifier_args(
        &prj,
        r#"
name=$(ls test/generated/run-1-agent-1)
confirmed='{"verdict": "confirmed", "reason": "Every disproof failed."}'
case "$name" in
Failing.t.sol) printf '%s' "$confirmed" > "$FORGE_PROPERTIES_VERDICT"; echo "quota exceeded" >&2; exit 3 ;;
Invalid.t.sol) printf '{"verdict": "maybe"}' > "$FORGE_PROPERTIES_VERDICT" ;;
NoEvidence.t.sol) printf '{"verdict": "rejected", "reason": "Not a bug."}' > "$FORGE_PROPERTIES_VERDICT" ;;
Slow.t.sol) printf '%s' "$confirmed" > "$FORGE_PROPERTIES_VERDICT"; sleep 30 ;;
esac
"#,
    ));
    cmd.args(args).args(["--agents", "3", "--timeout", "5"]).assert_success().stdout_eq(str![[r#"
confirmed (2/3 agents): @bug Fee.failing src/Fee.sol:7 — the failing is wrong.
  agent 1: FailingTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.failing src/Fee.sol:7 — the failing is wrong.
    file: cache/properties/run-1/agent-1/tests/Failing.t.sol
  agent 2: FailingTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.failing src/Fee.sol:7 — the failing is wrong.
    file: cache/properties/run-1/agent-2/tests/Failing.t.sol
  verifier: Every disproof failed.
  verifier error: verifier command failed (exit status: 3): see cache/properties/run-1/verify-1/verifier.log
confirmed (2/3 agents): @bug Fee.slow src/Fee.sol:7 — the slow is wrong.
  agent 1: SlowTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.slow src/Fee.sol:7 — the slow is wrong.
    file: cache/properties/run-1/agent-1/tests/Slow.t.sol
  agent 2: SlowTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.slow src/Fee.sol:7 — the slow is wrong.
    file: cache/properties/run-1/agent-2/tests/Slow.t.sol
  verifier: Every disproof failed.
  verifier error: the verifier reached the timeout: see cache/properties/run-1/verify-4/verifier.log
needs validation (2/3 agents): @bug Fee.invalid src/Fee.sol:7 — the invalid is wrong.
  agent 1: InvalidTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.invalid src/Fee.sol:7 — the invalid is wrong.
    file: cache/properties/run-1/agent-1/tests/Invalid.t.sol
  agent 2: InvalidTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.invalid src/Fee.sol:7 — the invalid is wrong.
    file: cache/properties/run-1/agent-2/tests/Invalid.t.sol
  verifier error: the verifier wrote an invalid verdict: unknown variant `maybe`, expected one of `confirmed`, `needs_validation`, `rejected` at line 1 column 19: see cache/properties/run-1/verify-2/verifier.log
needs validation (2/3 agents): @bug Fee.noevidence src/Fee.sol:7 — the noevidence is wrong.
  agent 1: NoEvidenceTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.noevidence src/Fee.sol:7 — the noevidence is wrong.
    file: cache/properties/run-1/agent-1/tests/NoEvidence.t.sol
  agent 2: NoEvidenceTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.noevidence src/Fee.sol:7 — the noevidence is wrong.
    file: cache/properties/run-1/agent-2/tests/NoEvidence.t.sol
  verifier: Not a bug.
  verifier error: the verifier rejected the group without evidence: see cache/properties/run-1/verify-3/verifier.log
1 possible bug(s) below the quorum of 2:
single-agent (1/3 agents): @bug Fee.single src/Fee.sol:7 — the single is wrong.
  agent 3: SingleTest::testBug (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: @bug Fee.single src/Fee.sol:7 — the single is wrong.
    file: cache/properties/run-1/agent-3/tests/Single.t.sol
no generated test file was kept

"#]]);

    assert_eq!(
        report(&prj)["possible_bugs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| group["verification"]["verdict"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["confirmed", "confirmed", "needs_validation", "needs_validation"]
    );
}

// When a generated file does not compile, each file that no other file imports runs with the
// files that it imports. A file that two of them import counts once, files that import each other
// in a cycle still run, and files that only a broken file imports run and are kept without it.
#[forgetest_init]
fn properties_runs_each_file_once_when_one_does_not_compile(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Broken.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import "./Kept.t.sol";
contract BrokenTest { function testFee() public { missing(); } }
SOL
cat > "$out/Kept.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import "./KeptHelper.sol";
contract KeptTest { function testKept() public pure { require(KeptHelper.one() == 1); } }
SOL
cat > "$out/KeptHelper.sol" <<'SOL'
pragma solidity ^0.8.20;
import "./Kept.t.sol";
library KeptHelper { function one() internal pure returns (uint256) { return 1; } }
SOL
cat > "$out/Shared.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Fee} from "../../../src/Fee.sol";
contract SharedTest {
    // The fee is capped at 100 ether (src/Fee.sol:7).
    function testCap() public pure { require(Fee.fee(1e30) == 100 ether); }
}
SOL
for name in First Second; do
cat > "$out/$name.t.sol" <<SOL
pragma solidity ^0.8.20;
import "./Shared.t.sol";
contract ${name}Test {}
SOL
done
cat > "$out/CycleA.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import "./CycleB.t.sol";
contract CycleATest { function testA() public pure { require(false); } }
SOL
cat > "$out/CycleB.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import "./CycleA.t.sol";
contract CycleBTest { function testB() public pure { require(false); } }
SOL
"#,
    );
    cmd.args(args).args(["--agents", "1"]).assert_success().stdout_eq(str![[r#"
possible bug (1/1 agents): The fee is capped at 100 ether (src/Fee.sol:7).
  agent 1: SharedTest::testCap (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    claim: The fee is capped at 100 ether (src/Fee.sol:7).
    file: cache/properties/run-1/agent-1/tests/Shared.t.sol
possible bug (1/1 agents): CycleATest::testA
  agent 1: CycleATest::testA (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    file: cache/properties/run-1/agent-1/tests/CycleA.t.sol
possible bug (1/1 agents): CycleBTest::testB
  agent 1: CycleBTest::testB (seed 24301: EvmError: Revert; seed 12648430: EvmError: Revert)
    file: cache/properties/run-1/agent-1/tests/CycleB.t.sol
1 generated test file(s) did not compile or were not allowed: see cache/properties/report.json
added 2 generated test file(s):
  test/generated/run-1-agent-1/Kept.t.sol
  test/generated/run-1-agent-1/KeptHelper.sol

"#]]);
}

// A file that another file of the same agent imports through a remapping stays, even when another
// agent wrote the same file, so the kept tests still compile.
#[forgetest_init]
fn properties_keeps_remapped_imports(prj: _, cmd: _) {
    prj.add_source("Fee.sol", FEE);
    prj.update_config(|config| {
        config.remappings.push(Remapping::from_str("generated/=test/generated/").unwrap().into());
    });
    let args = properties_args(
        &prj,
        r#"
cat > "$out/Helper.sol" <<'SOL'
pragma solidity ^0.8.20;
library Helper { function fee(uint256 amount) internal pure returns (uint256) { return amount / 100; } }
SOL
cat > "$out/Property.t.sol" <<SOL
pragma solidity ^0.8.20;
import {Fee} from "../../../src/Fee.sol";
import {Helper} from "generated/$(basename "$out")/Helper.sol";
contract Property${agent}Test { function testFee() public pure { require(Fee.fee(199) == Helper.fee(199)); } }
SOL
"#,
    );
    cmd.args(args).args(["--agents", "2"]).assert_success().stdout_eq(str![[r#"
added 4 generated test file(s):
  test/generated/run-1-agent-1/Helper.sol
  test/generated/run-1-agent-1/Property.t.sol
  test/generated/run-1-agent-2/Helper.sol
  test/generated/run-1-agent-2/Property.t.sol

"#]]);

    cmd.forge_fuse().args(["test", "--match-path", "test/generated/**"]).assert_success();
}

// Distinct string literals must not collapse under duplicate detection: whitespace inside a
// string and `//` inside a URL are significant, while comment-only differences still dedupe.
#[forgetest_init]
fn properties_keeps_tests_that_differ_only_inside_string_literals(prj: _, cmd: _) {
    prj.add_source(
        "Echo.sol",
        r#"
pragma solidity ^0.8.20;
library Echo {
    function echo(string memory value) internal pure returns (string memory) {
        return value;
    }
}
"#,
    );
    let args = properties_args(
        &prj,
        r#"
case "$1" in
whitespace)
  cat > "$out/Case1.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Echo} from "../../../src/Echo.sol";
contract EchoSpecTest {
  function testEcho() public pure {
    require(keccak256(bytes(Echo.echo("ab"))) == keccak256(bytes("ab")), "echo changed");
  }
}
SOL
  cat > "$out/Case2.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Echo} from "../../../src/Echo.sol";
contract EchoSpecTest {
  function testEcho() public pure {
    require(keccak256(bytes(Echo.echo("a b"))) == keccak256(bytes("a b")), "echo changed");
  }
}
SOL
  ;;
urls)
  cat > "$out/Case1.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Echo} from "../../../src/Echo.sol";
contract EchoSpecTest {
  function testEcho() public pure {
    require(keccak256(bytes(Echo.echo("https://one.example"))) == keccak256(bytes("https://one.example")), "echo changed");
  }
}
SOL
  cat > "$out/Case2.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Echo} from "../../../src/Echo.sol";
contract EchoSpecTest {
  function testEcho() public pure {
    require(keccak256(bytes(Echo.echo("https://two.example"))) == keccak256(bytes("https://two.example")), "echo changed");
  }
}
SOL
  ;;
duplicate)
  cat > "$out/Case1.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Echo} from "../../../src/Echo.sol";
// first
contract EchoSpecTest {
  function testEcho() public pure {
    require(keccak256(bytes(Echo.echo("ab"))) == keccak256(bytes("ab")), "echo changed");
  }
}
SOL
  cat > "$out/Case2.t.sol" <<'SOL'
pragma solidity ^0.8.20;
import {Echo} from "../../../src/Echo.sol";
// second
contract EchoSpecTest {
  function testEcho() public pure {
    require(keccak256(bytes(Echo.echo("ab"))) == keccak256(bytes("ab")), "echo changed");
  }
}
SOL
  ;;
*) exit 2 ;;
esac
"#,
    );

    for (mode, kept, duplicates) in [
        ("duplicate", 1usize, 1usize),
        ("whitespace", 2, 0),
        ("urls", 2, 0),
    ] {
        // Fresh generated dirs between modes.
        let generated = prj.root().join("test/generated");
        if generated.exists() {
            fs::remove_dir_all(&generated).unwrap();
        }
        let mut mode_args = args.clone();
        // properties_args already wrote agent.sh; pass the mode as the first agent arg.
        mode_args.extend(["--agent-arg".into(), mode.into(), "--agents".into(), "1".into()]);
        cmd.forge_fuse().args(&mode_args).assert_success();
        let report = report(&prj);
        assert_eq!(report["kept"].as_array().unwrap().len(), kept, "mode={mode}");
        assert_eq!(report["duplicates"].as_array().unwrap().len(), duplicates, "mode={mode}");
    }
}

// Concurrent campaigns on one project must not share prompt/usage paths under cache/properties.
#[forgetest_init]
fn properties_isolates_concurrent_campaign_session_files(prj: _, cmd: _) {
    prj.add_source("TargetA.sol", "pragma solidity ^0.8.20; contract TargetA {}");
    prj.add_source("TargetB.sol", "pragma solidity ^0.8.20; contract TargetB {}");
    let control = prj.root().join("concurrency-control");
    fs::create_dir_all(&control).unwrap();
    let agent = prj.root().join("agent.sh");
    fs::write(
        &agent,
        r#"#!/bin/sh
set -eu
mode="$1"
control="$2"
if [ "$mode" = slow ]; then
  : > "$control/ready"
  while [ ! -f "$control/release" ]; do sleep 0.05; done
  cp "$FORGE_PROPERTIES_PROMPT" "$control/observed-prompt.txt"
elif [ "$mode" = fast ]; then
  printf '{"input_tokens":22,"output_tokens":0,"cost_usd":0}\n' > "$FORGE_PROPERTIES_USAGE"
elif [ "$mode" = solo ]; then
  cp "$FORGE_PROPERTIES_PROMPT" "$control/solo-prompt.txt"
else
  exit 2
fi
"#,
    )
    .unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).unwrap();

    let forge = cmd.cmd().get_program().to_owned();
    let root = prj.root().to_path_buf();
    let common = |mode: &str, target: &str| {
        vec![
            "properties".into(),
            "--root".into(),
            root.to_string_lossy().into_owned(),
            "--target".into(),
            target.into(),
            "--agent-command".into(),
            agent.to_string_lossy().into_owned(),
            "--agent-arg".into(),
            mode.into(),
            "--agent-arg".into(),
            control.to_string_lossy().into_owned(),
            "--agents".into(),
            "1".into(),
            "--seed".into(),
            "1".into(),
            "--seed".into(),
            "2".into(),
            "--timeout".into(),
            "20".into(),
        ]
    };

    // Control: a solo campaign keeps TargetA in its prompt.
    cmd.forge_fuse().args(common("solo", "src/TargetA.sol")).assert_success();
    let solo = fs::read_to_string(control.join("solo-prompt.txt")).unwrap();
    assert!(solo.contains("src/TargetA.sol"), "{solo}");

    let forge_bin = forge.clone();
    let slow_args = common("slow", "src/TargetA.sol");
    let slow = thread::spawn(move || {
        std::process::Command::new(&forge_bin)
            .args(&slow_args)
            .env("FOUNDRY_DISABLE_NIGHTLY_WARNING", "1")
            .output()
            .unwrap()
    });

    let ready = Instant::now();
    while !control.join("ready").exists() {
        assert!(ready.elapsed() < Duration::from_secs(15), "slow agent never became ready");
        thread::sleep(Duration::from_millis(20));
    }

    cmd.forge_fuse().args(common("fast", "src/TargetB.sol")).assert_success();
    fs::write(control.join("release"), b"").unwrap();
    let slow_out = slow.join().unwrap();
    assert!(slow_out.status.success(), "stderr={}", String::from_utf8_lossy(&slow_out.stderr));

    let observed = fs::read_to_string(control.join("observed-prompt.txt")).unwrap();
    assert!(
        observed.contains("src/TargetA.sol") && !observed.contains("src/TargetB.sol"),
        "slow campaign lost its prompt isolation:\n{observed}"
    );
    // Two run caches must exist; the slow campaign is run-1, the fast one run-2 (or vice versa
    // only if reservation order flipped — both should be present).
    let properties = prj.root().join("cache/properties");
    let runs = fs::read_dir(&properties)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("run-"))
        .count();
    assert!(runs >= 2, "expected per-run caches, found {runs} under {}", properties.display());
}
