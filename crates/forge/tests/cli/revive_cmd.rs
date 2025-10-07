use std::{fs, path::Path, str::FromStr};

use foundry_compilers::artifacts::{ConfigurableContractArtifact, Metadata, Remapping};
use foundry_config::SolidityErrorCode;
use foundry_test_utils::{TestProject, snapbox::IntoData};

use crate::constants::*;
const CONTRACT_ARTIFACT_JSON: &str = "Foo.sol/Foo.json";
const CONTRACT_ARTIFACT_BASE: &str = "Foo.sol/Foo";

fn init_prj(prj: &TestProject) {
    prj.add_source(
        "Foo",
        r"
pragma solidity *;
contract Foo {}
   ",
    )
    .unwrap();
}

// checks that `clean` works
forgetest_init!(can_clean_config, |prj, cmd| {
    // Resolc does not respect the `out` settings, example:
    // prj.update_config(|config| config.out = "custom-out".into());
    cmd.args(["build", "--resolc"]).assert_success();

    let artifact = prj.artifacts().join(TEMPLATE_TEST_CONTRACT_ARTIFACT_JSON);
    assert!(artifact.exists());

    cmd.forge_fuse().arg("clean").assert_empty_stdout();
    assert!(!artifact.exists());
});

forgetest!(must_rebuild_when_used_the_same_out, |prj, cmd| {
    prj.add_raw_source(
        "Foo",
        r"
pragma solidity *;
contract Foo {}
   ",
    )
    .unwrap();

    // compile with solc
    cmd.args(["build"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    let artifact = prj.artifacts();
    assert!(artifact.exists());

    // compile with resolc to the same output dir (resolc has hardcoded output dir)
    cmd.forge_fuse().args(["build", "--resolc"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // compile again with solc to the same output dir
    cmd.forge_fuse().args(["build"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// checks that extra output works
// TODO: Failing with resolc 0.4.0. Works with resolc 0.3.0.
forgetest!(can_emit_extra_output_for_resolc, |prj, cmd| {
    prj.clear();
    init_prj(&prj);

    cmd.args(["build", "--resolc", "--use-resolc", "0.3.0", "--extra-output", "metadata"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    let artifact_path = prj.artifacts().join(CONTRACT_ARTIFACT_JSON);
    let artifact: ConfigurableContractArtifact =
        foundry_compilers::utils::read_json_file(&artifact_path).unwrap();
    assert!(artifact.metadata.is_some());

    cmd.forge_fuse()
        .args([
            "build",
            "--resolc",
            "--use-resolc",
            "0.3.0",
            "--extra-output-files",
            "metadata",
            "--force",
        ])
        .root_arg()
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    let metadata_path = prj.artifacts().join(format!("{CONTRACT_ARTIFACT_BASE}.metadata.json"));
    let _artifact: Metadata = foundry_compilers::utils::read_json_file(&metadata_path).unwrap();
});

// checks that extra output works
// TODO: Failing with resolc 0.4.0. Works with resolc 0.3.0.
forgetest!(can_emit_multiple_extra_output_for_resolc, |prj, cmd| {
    init_prj(&prj);
    cmd.args([
        "build",
        "--resolc",
        "--use-resolc",
        "0.3.0",
        "--extra-output",
        "metadata",
        "ir-optimized",
        "--extra-output",
        "ir",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    let artifact_path = prj.artifacts().join(CONTRACT_ARTIFACT_JSON);
    let artifact: ConfigurableContractArtifact =
        foundry_compilers::utils::read_json_file(&artifact_path).unwrap();
    assert!(artifact.metadata.is_some());
    assert!(artifact.ir.is_some());
    assert!(artifact.ir_optimized.is_some());

    cmd.forge_fuse()
        .args([
            "build",
            "--resolc",
            "--use-resolc",
            "0.3.0",
            "--extra-output-files",
            "metadata",
            "ir-optimized",
            "ir",
            "--force",
        ])
        .root_arg()
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    let metadata_path = prj.artifacts().join(format!("{CONTRACT_ARTIFACT_BASE}.metadata.json"));
    let _artifact: Metadata = foundry_compilers::utils::read_json_file(&metadata_path).unwrap();

    let iropt = prj.artifacts().join(format!("{CONTRACT_ARTIFACT_BASE}.iropt"));
    std::fs::read_to_string(iropt).unwrap();

    let ir = prj.artifacts().join(format!("{CONTRACT_ARTIFACT_BASE}.ir"));
    std::fs::read_to_string(ir).unwrap();
});

forgetest!(can_print_warnings_for_resolc, |prj, cmd| {
    prj.add_source(
        "Foo",
        r"
contract Greeter {
    function foo(uint256 a) public {
        uint256 x = 1;
    }
}
   ",
    )
    .unwrap();

    cmd.args(["build", "--resolc"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (5667): Unused function parameter. Remove or comment out the variable name to silence this warning.
 [FILE]:5:18:
  |
5 |     function foo(uint256 a) public {
  |                  ^^^^^^^^^

Warning (2072): Unused local variable.
 [FILE]:6:9:
  |
6 |         uint256 x = 1;
  |         ^^^^^^^^^

Warning (2018): Function state mutability can be restricted to pure
 [FILE]:5:5:
  |
5 |     function foo(uint256 a) public {
  |     ^ (Relevant source part starts here and spans across multiple lines).


"#]]);
});

// tests that the `inspect` command works correctly
forgetest!(can_execute_inspect_command_for_resolc, |prj, cmd| {
    let contract_name = "Foo";
    let path = prj
        .add_source(
            contract_name,
            r#"
contract Foo {
    event log_string(string);
    function run() external {
        emit log_string("script ran");
    }
}
    "#,
        )
        .unwrap();

    cmd.args(["inspect", "--resolc"])
        .arg(contract_name)
        .arg("bytecode")
        .assert_success()
        .stdout_eq(str![[r#"
0x50564d00[..]

"#]]);

    let info = format!("src/{}:{}", path.file_name().unwrap().to_string_lossy(), contract_name);
    cmd.forge_fuse()
        .args(["inspect", "--resolc"])
        .arg(info)
        .arg("bytecode")
        .assert_success()
        .stdout_eq(str![[r#"
0x50564d00[..]

"#]]);
});

// test that `forge build` does not print `(with warnings)` if file path is ignored
forgetest!(can_compile_without_warnings_ignored_file_paths_for_resolc, |prj, cmd| {
    // Ignoring path and setting empty error_codes as default would set would set some error codes
    prj.update_config(|config| {
        config.ignored_file_paths = vec![Path::new("src").to_path_buf()];
        config.ignored_error_codes = vec![];
    });

    prj.add_raw_source(
        "src/example.sol",
        r"
pragma solidity *;
contract A {
    function testExample() public {}
}
",
    )
    .unwrap();

    cmd.args(["build", "--resolc", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // Reconfigure without ignored paths or error codes and check for warnings
    prj.update_config(|config| config.ignored_file_paths = vec![]);

    cmd.forge_fuse().args(["build", "--resolc", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (1878): SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information.
Warning: SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information.
[FILE]


"#]]);
});

// test that `forge build` does not print `(with warnings)` if there aren't any
forgetest!(can_compile_without_warnings_for_resolc, |prj, cmd| {
    prj.update_config(|config| {
        config.ignored_error_codes = vec![SolidityErrorCode::SpdxLicenseNotProvided];
    });
    prj.add_raw_source(
        "A",
        r"
pragma solidity *;
contract A {
    function testExample() public {}
}
   ",
    )
    .unwrap();

    cmd.args(["build", "--resolc", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // don't ignore errors
    prj.update_config(|config| {
        config.ignored_error_codes = vec![];
    });

    cmd.forge_fuse().args(["build", "--resolc", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (1878): SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information.
Warning: SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information.
[FILE]


"#]]);
});

// test that `forge build` compiles when severity set to error, fails when set to warning, and
// handles ignored error codes as an exception
forgetest!(can_fail_compile_with_warnings_for_resolc, |prj, cmd| {
    prj.update_config(|config| {
        config.ignored_error_codes = vec![];
        config.deny_warnings = false;
    });
    prj.add_raw_source(
        "A",
        r"
pragma solidity *;
contract A {
    function testExample() public {}
}
   ",
    )
    .unwrap();

    // there are no errors
    cmd.args(["build", "--resolc", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (1878): SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information.
Warning: SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information.
[FILE]


"#]]);

    // warning fails to compile
    prj.update_config(|config| {
        config.ignored_error_codes = vec![];
        config.deny_warnings = true;
    });

    cmd.forge_fuse().args(["build", "--resolc", "--force"]).assert_failure().stderr_eq(str![[r#"
Error: Compiler run failed:
Warning (1878): SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information.
Warning: SPDX license identifier not provided in source file. Before publishing, consider adding a comment containing "SPDX-License-Identifier: <SPDX-License>" to each source file. Use "SPDX-License-Identifier: UNLICENSED" for non-open-source code. Please see https://spdx.org for more information.
[FILE]

"#]]);

    // ignores error code and compiles
    prj.update_config(|config| {
        config.ignored_error_codes = vec![SolidityErrorCode::SpdxLicenseNotProvided];
        config.deny_warnings = true;
    });

    cmd.forge_fuse().args(["build", "--resolc", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// test that a failing `forge build` does not impact followup builds
forgetest!(can_build_after_failure_for_resolc, |prj, cmd| {
    init_prj(&prj);

    cmd.args(["build", "--resolc"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    let artifact = prj.artifacts().join(CONTRACT_ARTIFACT_JSON);
    assert!(artifact.exists());
    let cache = prj.root().join("cache/resolc-solidity-files-cache.json");
    assert!(cache.exists());

    let syntax_err = r#"
pragma solidity *;
contract Foo {
    function foo() public {
        THIS WILL CAUSE AN ERROR
    }
}
   "#;

    // introduce contract with syntax error
    prj.add_source("Foo.sol", syntax_err).unwrap();

    // `forge build --force` which should fail
    cmd.forge_fuse().args(["build", "--resolc", "--force"]).assert_failure().stderr_eq(str![[r#"
Error: Compiler run failed:
Error (2314): Expected ';' but got identifier
 [FILE]:6:19:
  |
6 |         THIS WILL CAUSE AN ERROR
  |                   ^^^^^

"#]]);

    // but ensure this cleaned cache and artifacts
    assert!(!artifact.exists());
    assert!(!cache.exists());

    // still errors
    cmd.forge_fuse().args(["build", "--resolc", "--force"]).assert_failure().stderr_eq(str![[r#"
Error: Compiler run failed:
Error (2314): Expected ';' but got identifier
 [FILE]:6:19:
  |
6 |         THIS WILL CAUSE AN ERROR
  |                   ^^^^^

"#]]);

    // resolve the error by replacing the file
    prj.add_source(
        "Foo.sol",
        r#"
pragma solidity *;
contract Foo {
    function foo() public {
    }
}
   "#,
    )
    .unwrap();

    cmd.forge_fuse().args(["build", "--resolc", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    assert!(artifact.exists());
    assert!(cache.exists());

    // ensure cache is unchanged after error
    let cache_before = fs::read_to_string(&cache).unwrap();

    // introduce the error again but building without force
    prj.add_source("Foo.sol", syntax_err).unwrap();
    cmd.forge_fuse().args(["build", "--resolc"]).assert_failure().stderr_eq(str![[r#"
Error: Compiler run failed:
Error (2314): Expected ';' but got identifier
 [FILE]:6:19:
  |
6 |         THIS WILL CAUSE AN ERROR
  |                   ^^^^^

"#]]);

    // ensure unchanged cache file
    let cache_after = fs::read_to_string(cache).unwrap();
    assert_eq!(cache_before, cache_after);
});

// checks that extra output works
forgetest_init!(can_build_skip_contracts_for_resolc, |prj, cmd| {
    prj.clear();

    // Only builds the single template contract `src/*`
    cmd.args(["build", "--resolc", "--skip", "tests", "--skip", "scripts"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // Expect compilation to be skipped as no files have changed
    cmd.arg("build").assert_success().stdout_eq(str![[r#"
No files changed, compilation skipped

"#]]);
});

forgetest_init!(can_build_skip_glob_for_resolc, |prj, cmd| {
    prj.add_test(
        "Foo",
        r"
contract TestDemo {
function test_run() external {}
}",
    )
    .unwrap();

    // only builds the single template contract `src/*` even if `*.t.sol` or `.s.sol` is absent
    prj.clear();
    cmd.args(["build", "--resolc", "--skip", "*/test/**", "--skip", "*/script/**", "--force"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    cmd.forge_fuse()
        .args(["build", "--resolc", "--skip", "./test/**", "--skip", "./script/**", "--force"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

forgetest_init!(can_build_specific_paths_for_resolc, |prj, cmd| {
    prj.wipe();
    prj.add_source(
        "Counter.sol",
        r"
contract Counter {
function count() external {}
}",
    )
    .unwrap();
    prj.add_test(
        "Foo.sol",
        r"
contract Foo {
function test_foo() external {}
}",
    )
    .unwrap();
    prj.add_test(
        "Bar.sol",
        r"
contract Bar {
function test_bar() external {}
}",
    )
    .unwrap();

    // Build 2 files within test dir
    prj.clear();
    cmd.args(["build", "--resolc", "test", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // Build one file within src dir
    prj.clear();
    cmd.forge_fuse();
    cmd.args(["build", "--resolc", "src", "--force"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // Build 3 files from test and src dirs
    prj.clear();
    cmd.forge_fuse();
    cmd.args(["build", "--resolc", "src", "test", "--force"]).assert_success().stdout_eq(str![[
        r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#
    ]]);

    // Build single test file
    prj.clear();
    cmd.forge_fuse();
    cmd.args(["build", "--resolc", "test/Bar.sol", "--force"]).assert_success().stdout_eq(str![[
        r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#
    ]]);

    // Fail if no source file found.
    prj.clear();
    cmd.forge_fuse();
    cmd.args(["build", "--resolc", "test/Dummy.sol", "--force"]).assert_failure().stderr_eq(str![
        [r#"
Error: No source files found in specified build paths.

"#]
    ]);
});

// checks that build --sizes includes all contracts even if unchanged
forgetest!(can_build_sizes_repeatedly_for_resolc, |prj, cmd| {
    init_prj(&prj);

    cmd.args(["build", "--resolc", "--sizes", "--use-resolc", "resolc:0.1.0-dev.16"])
        .assert_success()
        .stdout_eq(str![[r#"
...
╭----------+------------------+-------------------+--------------------+---------------------╮
| Contract | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+============================================================================================+
| Foo      | 1,288            | 1,288             | 248,712            | 248,712             |
╰----------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse()
        .args(["build", "--resolc", "--sizes", "--json", "--use-resolc", "resolc:0.1.0-dev.16"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "Foo": {
    "runtime_size": 1288,
    "init_size": 1288,
    "runtime_margin": 248712,
    "init_margin": 248712
  }
}
"#]]
            .is_json(),
        );
});

// checks that build --names includes all contracts even if unchanged
forgetest!(can_build_names_repeatedly_for_resolc, |prj, cmd| {
    init_prj(&prj);

    cmd.args(["build", "--resolc", "--names"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!
  compiler version: [..]
    - [..]
...

"#]]);

    cmd.forge_fuse()
        .args(["build", "--resolc", "--names", "--json"])
        .assert_success()
        .stdout_eq(str![[r#""{...}""#]].is_json());
});

forgetest_init!(can_inspect_counter_pretty_for_resolc, |prj, cmd| {
    cmd.args(["inspect", "--resolc", "src/Counter.sol:Counter", "abi"]).assert_success().stdout_eq(
        str![[r#"

╭----------+---------------------------------+------------╮
| Type     | Signature                       | Selector   |
+=========================================================+
| function | increment() nonpayable          | 0xd09de08a |
|----------+---------------------------------+------------|
| function | number() view returns (uint256) | 0x8381f58a |
|----------+---------------------------------+------------|
| function | setNumber(uint256) nonpayable   | 0x3fb5c1cb |
╰----------+---------------------------------+------------╯


"#]],
    );
});

const CUSTOM_COUNTER: &str = r#"
    contract Counter {
    uint256 public number;
    uint64 public count;
    struct MyStruct {
        uint64 count;
    }
    struct ErrWithMsg {
        string message;
    }

    event Incremented(uint256 newValue);
    event Decremented(uint256 newValue);

    error NumberIsZero();
    error CustomErr(ErrWithMsg e);

    constructor(uint256 _number) {
        number = _number;
    }

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }

    function increment() external {
        number++;
    }

    function decrement() public payable {
        if (number == 0) {
            return;
        }
        number--;
    }

    function square() public {
        number = number * number;
    }

    fallback() external payable {
        ErrWithMsg memory err = ErrWithMsg("Fallback function is not allowed");
        revert CustomErr(err);
    }

    receive() external payable {
        count++;
    }

    function setStruct(MyStruct memory s, uint32 b) public {
        count = s.count;
    }
}
    "#;

const ANOTHER_COUNTER: &str = r#"
    contract AnotherCounter is Counter {
        constructor(uint256 _number) Counter(_number) {}
    }
"#;
forgetest!(inspect_custom_counter_abi_for_resolc, |prj, cmd| {
    prj.add_source("Counter.sol", CUSTOM_COUNTER).unwrap();

    cmd.args(["inspect", "--resolc", "Counter", "abi"]).assert_success().stdout_eq(str![[r#"

╭-------------+-----------------------------------------------+--------------------------------------------------------------------╮
| Type        | Signature                                     | Selector                                                           |
+==================================================================================================================================+
| event       | Decremented(uint256)                          | 0xc9118d86370931e39644ee137c931308fa3774f6c90ab057f0c3febf427ef94a |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| event       | Incremented(uint256)                          | 0x20d8a6f5a693f9d1d627a598e8820f7a55ee74c183aa8f1a30e8d4e8dd9a8d84 |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| error       | CustomErr(Counter.ErrWithMsg)                 | 0x0625625a                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| error       | NumberIsZero()                                | 0xde5d32ac                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| function    | count() view returns (uint64)                 | 0x06661abd                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| function    | decrement() payable                           | 0x2baeceb7                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| function    | increment() nonpayable                        | 0xd09de08a                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| function    | number() view returns (uint256)               | 0x8381f58a                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| function    | setNumber(uint256) nonpayable                 | 0x3fb5c1cb                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| function    | setStruct(Counter.MyStruct,uint32) nonpayable | 0x08ef7366                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| function    | square() nonpayable                           | 0xd742cb01                                                         |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| constructor | constructor(uint256) nonpayable               |                                                                    |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| fallback    | fallback() payable                            |                                                                    |
|-------------+-----------------------------------------------+--------------------------------------------------------------------|
| receive     | receive() payable                             |                                                                    |
╰-------------+-----------------------------------------------+--------------------------------------------------------------------╯


"#]]);
});

forgetest!(inspect_custom_counter_events_for_resolc, |prj, cmd| {
    prj.add_source("Counter.sol", CUSTOM_COUNTER).unwrap();

    cmd.args(["inspect", "--resolc", "Counter", "events"]).assert_success().stdout_eq(str![[r#"

╭----------------------+--------------------------------------------------------------------╮
| Event                | Topic                                                              |
+===========================================================================================+
| Decremented(uint256) | 0xc9118d86370931e39644ee137c931308fa3774f6c90ab057f0c3febf427ef94a |
|----------------------+--------------------------------------------------------------------|
| Incremented(uint256) | 0x20d8a6f5a693f9d1d627a598e8820f7a55ee74c183aa8f1a30e8d4e8dd9a8d84 |
╰----------------------+--------------------------------------------------------------------╯


"#]]);
});

forgetest!(inspect_custom_counter_errors_for_resolc, |prj, cmd| {
    prj.add_source("Counter.sol", CUSTOM_COUNTER).unwrap();

    cmd.args(["inspect", "--resolc", "Counter", "errors"]).assert_success().stdout_eq(str![[r#"

╭-------------------------------+----------╮
| Error                         | Selector |
+==========================================+
| CustomErr(Counter.ErrWithMsg) | 0625625a |
|-------------------------------+----------|
| NumberIsZero()                | de5d32ac |
╰-------------------------------+----------╯


"#]]);
});

forgetest!(inspect_path_only_identifier_for_resolc, |prj, cmd| {
    prj.add_source("Counter.sol", CUSTOM_COUNTER).unwrap();

    cmd.args(["inspect", "--resolc", "src/Counter.sol", "errors"]).assert_success().stdout_eq(
        str![[r#"

╭-------------------------------+----------╮
| Error                         | Selector |
+==========================================+
| CustomErr(Counter.ErrWithMsg) | 0625625a |
|-------------------------------+----------|
| NumberIsZero()                | de5d32ac |
╰-------------------------------+----------╯


"#]],
    );
});

forgetest!(test_inspect_contract_with_same_name_for_resolc, |prj, cmd| {
    let source = format!("{CUSTOM_COUNTER}\n{ANOTHER_COUNTER}");
    prj.add_source("Counter.sol", &source).unwrap();

    cmd.args(["inspect", "--resolc", "src/Counter.sol", "errors"]).assert_failure().stderr_eq(str![[r#"
Error: Multiple contracts found in the same file, please specify the target <path>:<contract> or <contract>

"#]]);

    cmd.forge_fuse().args(["inspect", "--resolc", "Counter", "errors"]).assert_success().stdout_eq(
        str![[r#"

╭-------------------------------+----------╮
| Error                         | Selector |
+==========================================+
| CustomErr(Counter.ErrWithMsg) | 0625625a |
|-------------------------------+----------|
| NumberIsZero()                | de5d32ac |
╰-------------------------------+----------╯


"#]],
    );
});

// TODO: Failing with resolc 0.4.0. Works with resolc 0.3.0.
forgetest!(inspect_custom_counter_method_identifiers_for_resolc, |prj, cmd| {
    prj.add_source("Counter.sol", CUSTOM_COUNTER).unwrap();

    cmd.args(["inspect", "--resolc", "--use-resolc", "0.3.0", "Counter", "method-identifiers"])
        .assert_success()
        .stdout_eq(str![[r#"

╭----------------------------+------------╮
| Method                     | Identifier |
+=========================================+
| count()                    | 06661abd   |
|----------------------------+------------|
| decrement()                | 2baeceb7   |
|----------------------------+------------|
| increment()                | d09de08a   |
|----------------------------+------------|
| number()                   | 8381f58a   |
|----------------------------+------------|
| setNumber(uint256)         | 3fb5c1cb   |
|----------------------------+------------|
| setStruct((uint64),uint32) | 08ef7366   |
|----------------------------+------------|
| square()                   | d742cb01   |
╰----------------------------+------------╯


"#]]);
});

// checks forge bind works correctly on the default project
forgetest!(can_bind_for_resolc, |prj, cmd| {
    init_prj(&prj);

    cmd.args(["bind", "--resolc"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!
Generating bindings for [..] contracts
Bindings have been generated to [..]

"#]]);
});

forgetest!(can_bind_enum_modules_for_resolc, |prj, cmd| {
    prj.clear();

    prj.add_source(
        "Enum.sol",
        r#"
    contract Enum {
        enum MyEnum { A, B, C }
    }
    "#,
    )
    .unwrap();

    prj.add_source(
        "UseEnum.sol",
        r#"
    import "./Enum.sol";
    contract UseEnum {
        Enum.MyEnum public myEnum;
    }"#,
    )
    .unwrap();

    //TODO: bind command is looking for artifacts in wrong directory
    cmd.args(["bind", "--resolc", "--select", "^Enum$"]).assert_success().stdout_eq(str![[
        r#"[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!
Generating bindings for 1 contracts
Bindings have been generated to [..]"#
    ]]);
});

// Tests that direct import paths are handled correctly
forgetest!(can_handle_direct_imports_into_src_for_resolc, |prj, cmd| {
    prj.add_source(
        "Foo",
        r#"
import {FooLib} from "src/FooLib.sol";
struct Bar {
    uint8 x;
}
contract Foo {
    mapping(uint256 => Bar) bars;
    function checker(uint256 id) external {
        Bar memory b = bars[id];
        FooLib.check(b);
    }
    function checker2() external {
        FooLib.check2(this);
    }
}
   "#,
    )
    .unwrap();

    prj.add_source(
        "FooLib",
        r#"
import {Foo, Bar} from "src/Foo.sol";
library FooLib {
    function check(Bar memory b) internal {}
    function check2(Foo f) internal {}
}
   "#,
    )
    .unwrap();

    cmd.args(["build", "--resolc"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

forgetest!(can_use_absolute_imports_for_resolc, |prj, cmd| {
    prj.update_config(|config| {
        let remapping = prj.paths().libraries[0].join("myDependency");
        config.remappings = vec![
            Remapping::from_str(&format!("myDependency/={}", remapping.display())).unwrap().into(),
        ];
    });

    prj.add_lib(
        "myDependency/src/interfaces/IConfig.sol",
        r"
    interface IConfig {}
   ",
    )
    .unwrap();

    prj.add_lib(
        "myDependency/src/Config.sol",
        r#"
        import "src/interfaces/IConfig.sol";
    contract Config {}
   "#,
    )
    .unwrap();

    prj.add_source(
        "Greeter",
        r#"
        import "myDependency/src/Config.sol";
    contract Greeter {}
   "#,
    )
    .unwrap();

    cmd.args(["build", "--resolc"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});
