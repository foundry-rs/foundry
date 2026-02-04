use crate::utils::generate_large_init_contract;
use foundry_test_utils::{forgetest, forgetest_init, snapbox::IntoData, str};
use globset::Glob;
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

note[multi-contract-file]: prefer having only one contract, interface or library per file
 [FILE]:4:10
  |
4 | contract ContractA {
  |          ^^^^^^^^^
  |
  = help: https://book.getfoundry.sh/reference/forge/forge-lint#multi-contract-file

note[multi-contract-file]: prefer having only one contract, interface or library per file
  [FILE]:12:10
   |
12 | contract ContractB {
   |          ^^^^^^^^^
   |
   = help: https://book.getfoundry.sh/reference/forge/forge-lint#multi-contract-file


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
