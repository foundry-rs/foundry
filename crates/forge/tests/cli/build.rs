use crate::utils::generate_large_init_contract;
#[cfg(feature = "monad")]
use foundry_evm_networks::NetworkConfigs;
use foundry_test_utils::{forgetest, forgetest_init, snapbox::IntoData, str};
use globset::Glob;
use serde_json::Value;
use std::fs;

forgetest_init!(can_parse_build_filters, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.clear();

    cmd.args(["build", "--names", "--skip", "tests", "scripts"]).assert_success().stdout_eq(str![
        [r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
  compiler version: [..]
    - Counter

"#]
    ]);
});

forgetest!(throws_on_conflicting_args, |prj, cmd| {
    prj.clear();

    cmd.args(["compile", "--format-json", "--quiet"]).assert_failure().stderr_eq(str![[r#"
error: the argument '--json' cannot be used with '--quiet'

Usage: forge[..] build --json [PATHS]...

For more information, try '--help'.

"#]]);
});

// tests that json is printed when --format-json is passed
forgetest!(compile_json, |prj, cmd| {
    prj.add_source(
        "jsonError",
        r"
contract Dummy {
    uint256 public number;
    function something(uint256 newNumber) public {
        number = newnumber; // error here
    }
}
",
    );

    // set up command
    cmd.args(["compile", "--format-json"]).assert_success().stderr_eq("").stdout_eq(str![[r#"
{
  "errors": [
    {
      "sourceLocation": {
        "file": "src/jsonError.sol",
        "start": 184,
        "end": 193
      },
      "type": "DeclarationError",
      "component": "general",
      "severity": "error",
      "errorCode": "7576",
      "message": "Undeclared identifier. Did you mean \"newNumber\"?",
      "formattedMessage": "DeclarationError: Undeclared identifier. Did you mean \"newNumber\"?\n [FILE]:7:18:\n  |\n7 |         number = newnumber; // error here\n  |                  ^^^^^^^^^\n\n"
    }
  ],
  "sources": {},
  "contracts": {},
  "build_infos": "{...}"
}
"#]].is_json());
});

forgetest!(initcode_size_exceeds_limit, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str());
    cmd.args(["build", "--sizes"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

╭---------------+------------------+-------------------+--------------------+---------------------╮
| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+=================================================================================================+
| LargeContract | 62               | 50,125            | 24,514             | -973                |
╰---------------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--json"]).assert_failure().stdout_eq(
        str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 24514,
    "init_margin": -973
  }
}
"#]]
        .is_json(),
    );

    cmd.forge_fuse().args(["build", "--sizes", "--md"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
|---------------|------------------|-------------------|--------------------|---------------------|
| LargeContract | 62               | 50,125            | 24,514             | -973                |


"#]]);

    // Ignore EIP-3860

    cmd.forge_fuse().args(["build", "--sizes", "--ignore-eip-3860"]).assert_success().stdout_eq(
        str![[r#"
No files changed, compilation skipped

╭---------------+------------------+-------------------+--------------------+---------------------╮
| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+=================================================================================================+
| LargeContract | 62               | 50,125            | 24,514             | -973                |
╰---------------+------------------+-------------------+--------------------+---------------------╯


"#]],
    );

    cmd.forge_fuse()
        .args(["build", "--sizes", "--ignore-eip-3860", "--json"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 24514,
    "init_margin": -973
  }
}
"#]]
            .is_json(),
        );

    cmd.forge_fuse()
        .args(["build", "--sizes", "--ignore-eip-3860", "--md"])
        .assert_success()
        .stdout_eq(str![[r#"
No files changed, compilation skipped

| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
|---------------|------------------|-------------------|--------------------|---------------------|
| LargeContract | 62               | 50,125            | 24,514             | -973                |


"#]]);
});

forgetest!(build_sizes_respects_configured_code_size_limit, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str());
    prj.update_config(|config| {
        config.code_size_limit = Some(64_000);
    });

    cmd.args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 63938,
    "init_margin": 77875
  }
}
"#]]
        .is_json(),
    );
});

#[cfg(feature = "monad")]
forgetest!(build_sizes_respects_monad_network_code_size_limit, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str());
    prj.update_config(|config| {
        config.networks = NetworkConfigs::with_monad();
    });

    cmd.args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "LargeContract": {
    "runtime_size": 62,
    "init_size": 50125,
    "runtime_margin": 131010,
    "init_margin": 212019
  }
}
"#]]
        .is_json(),
    );
});

// tests build output is as expected
forgetest_init!(exact_build_output, |prj, cmd| {
    prj.initialize_default_contracts();
    cmd.args(["build", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// tests build output is as expected
forgetest_init!(build_sizes_no_forge_std, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.update_config(|config| {
        config.solc = Some(foundry_config::SolcReq::Version(semver::Version::new(0, 8, 27)));
    });

    cmd.args(["build", "--sizes"]).assert_success().stdout_eq(str![[r#"
...

╭----------+------------------+-------------------+--------------------+---------------------╮
| Contract | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+============================================================================================+
| Counter  | 481              | 509               | 24,095             | 48,643              |
╰----------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
  "Counter": {
    "runtime_size": 481,
    "init_size": 509,
    "runtime_margin": 24095,
    "init_margin": 48643
  }
}
"#]]
        .is_json(),
    );

    cmd.forge_fuse().args(["build", "--sizes", "--md"]).assert_success().stdout_eq(str![[r#"
...

| Contract | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
|----------|------------------|-------------------|--------------------|---------------------|
| Counter  | 481              | 509               | 24,095             | 48,643              |


"#]]);
});

// tests build output --sizes handles multiple contracts with the same name
forgetest_init!(build_sizes_multiple_contracts, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.add_source(
        "Foo",
        r"
contract Foo {
}
",
    );

    prj.add_source(
        "a/Counter",
        r"
contract Counter {
    uint256 public count;
    function increment() public {
        count++;
    }
}
",
    );

    prj.add_source(
        "b/Counter",
        r"
contract Counter {
    uint256 public count;
    function decrement() public {
        count--;
    }
}
",
    );

    cmd.args(["build", "--sizes"]).assert_success().stdout_eq(str![[r#"
...

╭-----------------------------+------------------+-------------------+--------------------+---------------------╮
| Contract                    | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+===============================================================================================================+
| Counter (src/Counter.sol)   | 481              | 509               | 24,095             | 48,643              |
|-----------------------------+------------------+-------------------+--------------------+---------------------|
| Counter (src/a/Counter.sol) | 344              | 372               | 24,232             | 48,780              |
|-----------------------------+------------------+-------------------+--------------------+---------------------|
| Counter (src/b/Counter.sol) | 291              | 319               | 24,285             | 48,833              |
|-----------------------------+------------------+-------------------+--------------------+---------------------|
| Foo                         | 62               | 88                | 24,514             | 49,064              |
╰-----------------------------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse().args(["build", "--sizes", "--md"]).assert_success().stdout_eq(str![[r#"
...

| Contract                    | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
|-----------------------------|------------------|-------------------|--------------------|---------------------|
| Counter (src/Counter.sol)   | 481              | 509               | 24,095             | 48,643              |
| Counter (src/a/Counter.sol) | 344              | 372               | 24,232             | 48,780              |
| Counter (src/b/Counter.sol) | 291              | 319               | 24,285             | 48,833              |
| Foo                         | 62               | 88                | 24,514             | 49,064              |


"#]]);
});

// tests build output --sizes --json handles multiple contracts with the same name
forgetest_init!(build_sizes_multiple_contracts_json, |prj, cmd| {
    prj.initialize_default_contracts();
    prj.add_source(
        "Foo",
        r"
contract Foo {
}
",
    );

    prj.add_source(
        "a/Counter",
        r"
contract Counter {
    uint256 public count;
    function increment() public {
        count++;
    }
}
",
    );

    prj.add_source(
        "b/Counter",
        r"
contract Counter {
    uint256 public count;
    function decrement() public {
        count--;
    }
}
",
    );

    cmd.args(["build", "--sizes", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
   "Counter (src/Counter.sol)":{
      "runtime_size":481,
      "init_size":509,
      "runtime_margin":24095,
      "init_margin":48643
   },
   "Counter (src/a/Counter.sol)":{
      "runtime_size":344,
      "init_size":372,
      "runtime_margin":24232,
      "init_margin":48780
   },
   "Counter (src/b/Counter.sol)":{
      "runtime_size":291,
      "init_size":319,
      "runtime_margin":24285,
      "init_margin":48833
   },
   "Foo":{
      "runtime_size":62,
      "init_size":88,
      "runtime_margin":24514,
      "init_margin":49064
   }
}
"#]]
        .is_json(),
    );
});

// tests that skip key in config can be used to skip non-compilable contract
forgetest_init!(test_can_skip_contract, |prj, cmd| {
    prj.add_source(
        "InvalidContract",
        r"
contract InvalidContract {
    some_invalid_syntax
}
",
    );

    prj.add_source(
        "ValidContract",
        r"
contract ValidContract {}
",
    );

    prj.update_config(|config| {
        config.skip = vec![Glob::new("src/InvalidContract.sol").unwrap().into()];
    });

    cmd.args(["build"]).assert_success();
});

// <https://github.com/foundry-rs/foundry/issues/11149>
forgetest_init!(test_consistent_build_output, |prj, cmd| {
    prj.add_source(
        "AContract.sol",
        r#"
import {B} from "/badpath/B.sol";

contract A is B {}
   "#,
    );

    prj.add_source(
        "CContract.sol",
        r#"
import {B} from "badpath/B.sol";

contract C is B {}
   "#,
    );

    cmd.args(["build", "src/AContract.sol"]).assert_failure().stdout_eq(str![[r#"
...
Unable to resolve imports:
      "/badpath/B.sol" in "[..]"
with remappings:
      forge-std/=[..]
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]

"#]]);
    cmd.forge_fuse().args(["build", "src/CContract.sol"]).assert_failure().stdout_eq(str![[r#"
Unable to resolve imports:
      "badpath/B.sol" in "[..]"
with remappings:
      forge-std/=[..]
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]

"#]]);
});

// <https://github.com/foundry-rs/foundry/issues/12458>
// <https://github.com/foundry-rs/foundry/issues/12496>
forgetest!(build_with_invalid_natspec, |prj, cmd| {
    prj.add_source(
        "ContractWithInvalidNatspec.sol",
        r#"
contract ContractA {
    /// @deprecated quoteExactOutputSingle and exactOutput. Use QuoterV2 instead.
}

/// Some editors highlight `@note` or `@todo`
/// @note foo bar

/// @title ContractB
contract ContractB {
    /**
    some example code in a comment:
    import { Ownable } from "@openzeppelin/contracts/access/Ownable.sol";
    */
}
   "#,
    );

    cmd.args(["build", "src/ContractWithInvalidNatspec.sol"]).assert_success().stderr_eq(str![[
        r#"
warning: invalid natspec tag '@deprecated', custom tags must use format '@custom:name'
  [FILE]:5:5
  │
5 │     /// @deprecated quoteExactOutputSingle and exactOutput. Use QuoterV2 instead.
  │     ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  │
...

warning: invalid natspec tag '@note', custom tags must use format '@custom:name'
  [FILE]:9:1
  │
9 │ /// @note foo bar
  │ ━━━━━━━━━━━━━━━━━
  │
...

"#
    ]]);
});

// tests that build succeeds without warning when no soldeer.lock exists
forgetest_init!(build_no_warning_without_soldeer_lock, |prj, cmd| {
    let soldeer_lock = prj.root().join("soldeer.lock");
    // soldeer.lock should not exist in a fresh project
    assert!(!soldeer_lock.exists());

    cmd.args(["build"]).assert_success().stderr_eq(str![[r#"
"#]]);
});

// tests that malformed foundry.lock triggers a warning during build
forgetest_init!(build_warns_on_malformed_foundry_lock, |prj, cmd| {
    let foundry_lock = prj.root().join("foundry.lock");
    fs::write(&foundry_lock, "this is not valid toml { [ }").unwrap();

    cmd.args(["build"]).assert_success().stderr_eq(str![[r#"
Warning: Failed to parse foundry.lock: [..]
...
"#]]);
});

// tests that build succeeds without warning when no foundry.lock exists
forgetest_init!(build_no_warning_without_foundry_lock, |prj, cmd| {
    let foundry_lock = prj.root().join("foundry.lock");
    // Remove foundry.lock if it exists from template
    let _ = fs::remove_file(&foundry_lock);

    cmd.args(["build"]).assert_success().stderr_eq(str![[r#"
"#]]);
});

// `forge --machine build` emits a single envelope on stdout and nothing on stderr.
forgetest_init!(machine_mode_emits_envelope, |prj, cmd| {
    prj.initialize_default_contracts();
    let assert = cmd.args(["--machine", "build", "--force"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "expected empty stderr under --machine, got: {stderr}");
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["success"], true);
    assert!(envelope["data"]["artifacts"].as_u64().is_some(), "missing artifacts: {envelope}");
    assert!(envelope["data"]["errors"].as_u64().is_some(), "missing errors: {envelope}");
    assert!(envelope["data"]["warnings"].as_u64().is_some(), "missing warnings: {envelope}");
    assert!(envelope["data"]["unchanged"].as_bool().is_some(), "missing unchanged: {envelope}");
    assert_eq!(envelope["errors"], serde_json::json!([]));
    assert_eq!(envelope["warnings"], serde_json::json!([]));
});

// `--machine` rejects flags that would corrupt the envelope-only stdout
// contract. Asserts the stable `code` + exit code, not just message text.
forgetest_init!(machine_mode_rejects_unsupported_flags, |prj, cmd| {
    prj.initialize_default_contracts();
    let assert = cmd.args(["--machine", "build", "--names"]).assert_failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim()).expect("error envelope on stdout");

    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    assert_eq!(assert.get_output().status.code(), Some(2));
    let msg = envelope["errors"][0]["message"].as_str().unwrap_or("");
    assert!(msg.contains("--names"), "missing --names mention: {envelope}");
    assert_eq!(
        envelope["errors"][0]["details"]["unsupported_flags"],
        serde_json::json!(["--names"]),
        "missing structured unsupported_flags details: {envelope}"
    );
});

// `--quiet` must not suppress the machine envelope.
forgetest_init!(machine_mode_envelope_survives_quiet, |prj, cmd| {
    prj.initialize_default_contracts();
    let assert = cmd.args(["--machine", "--quiet", "build", "--force"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .expect("stdout is exactly one JSON envelope, even under --quiet");

    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["success"], true);
});

// `--machine` refuses configs where lint-on-build + non-`Never` deny would diverge
// human and machine success outcomes.
forgetest_init!(machine_mode_rejects_lint_deny_divergence, |prj, cmd| {
    prj.initialize_default_contracts();
    let toml = "\
[profile.default]\n\
deny = \"warnings\"\n\
[lint]\n\
lint_on_build = true\n\
";
    std::fs::write(prj.root().join("foundry.toml"), toml).unwrap();
    let assert = cmd.args(["--machine", "build", "--force"]).assert_failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim()).expect("error envelope on stdout");

    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    assert_eq!(assert.get_output().status.code(), Some(2));
});

// `--machine` rejects `--watch` even though the watch path normally short-circuits before
// `BuildArgs::run`.
forgetest_init!(machine_mode_rejects_watch, |prj, cmd| {
    prj.initialize_default_contracts();
    let assert = cmd.args(["--machine", "build", "--watch"]).assert_failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim()).expect("error envelope on stdout");

    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    assert_eq!(assert.get_output().status.code(), Some(2));
    let msg = envelope["errors"][0]["message"].as_str().unwrap_or("");
    assert!(msg.contains("--watch"), "missing --watch mention: {envelope}");
});

// Compile failures under `--machine` emit a typed `compiler.solc.error` envelope and exit `Build
// (4)`.
forgetest!(machine_mode_compile_failure_emits_typed_envelope, |prj, cmd| {
    prj.add_source(
        "BadSyntax",
        r"
contract Dummy {
    uint256 public number;
    function something(uint256 newNumber) public {
        number = newnumber;
    }
}
",
    );

    let assert = cmd.args(["--machine", "build", "--force"]).assert_failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim()).expect("failure envelope on stdout");

    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["data"], serde_json::Value::Null);
    let errors = envelope["errors"].as_array().expect("errors array");
    assert!(!errors.is_empty(), "expected at least one error: {envelope}");
    assert_eq!(errors[0]["code"], "compiler.solc.error");
    assert_eq!(assert.get_output().status.code(), Some(4));
});

// Empty project under `--machine` emits a success envelope (artifacts=0), not "Nothing to compile".
forgetest!(machine_mode_empty_project_emits_envelope, |_prj, cmd| {
    let assert = cmd.args(["--machine", "build"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value =
        serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON envelope");

    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["success"], true);
    assert_eq!(envelope["data"]["artifacts"], 0);
    assert_eq!(envelope["data"]["errors"], 0);
    assert_eq!(envelope["data"]["warnings"], 0);
    assert_eq!(envelope["errors"], serde_json::json!([]));
    assert_eq!(envelope["warnings"], serde_json::json!([]));
    assert_eq!(assert.get_output().status.code(), Some(0));
});

// tests that build warns when foundry.lock revision differs from actual submodule revision
forgetest_init!(build_warns_on_foundry_lock_revision_mismatch, |prj, cmd| {
    let foundry_lock = prj.root().join("foundry.lock");

    // Write a foundry.lock with a fake/old revision for forge-std that differs from the actual
    let lockfile_content = r#"{
  "lib/forge-std": {
    "tag": {
      "name": "v1.9.7",
      "rev": "0000000000000000000000000000000000000000"
    }
  }
}"#;
    fs::write(&foundry_lock, lockfile_content).unwrap();

    cmd.args(["build"]).assert_success().stderr_eq(str![[r#"
Warning: Dependency 'lib/forge-std' revision mismatch: expected '0000000000000000000000000000000000000000', found '[..]'

"#]]);
});
