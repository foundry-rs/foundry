//! Integration tests for `forge fmt` command

use foundry_test_utils::{forgetest, forgetest_init};

const UNFORMATTED: &str = r#"// SPDX-License-Identifier: MIT
pragma         solidity  =0.8.30    ;

contract  Test  {
    uint256    public    value ;
    function   setValue ( uint256   _value )   public   {
        value   =   _value ;
    }
}"#;

const FORMATTED: &str = r#"// SPDX-License-Identifier: MIT
pragma solidity =0.8.30;

contract Test {
    uint256 public value;

    function setValue(uint256 _value) public {
        value = _value;
    }
}
"#;

forgetest_init!(fmt_exclude_libs_in_recursion, |prj, cmd| {
    prj.update_config(|config| config.fmt.ignore = vec!["src/ignore/".to_string()]);

    prj.add_lib("SomeLib.sol", UNFORMATTED);
    prj.add_raw_source("ignore/IgnoredContract.sol", UNFORMATTED);
    cmd.args(["fmt", ".", "--check"]);
    cmd.assert_success();

    cmd.forge_fuse().args(["fmt", "lib/SomeLib.sol", "--check"]);
    cmd.assert_failure();
});

// Test that fmt can format a simple contract file
forgetest_init!(fmt_file, |prj, cmd| {
    prj.add_raw_source("FmtTest.sol", UNFORMATTED);
    cmd.arg("fmt").arg("src/FmtTest.sol");
    cmd.assert_success().stdout_eq(str![[r#"
Formatted [..]/src/FmtTest.sol

"#]]);
    assert_data_eq!(
        std::fs::read_to_string(prj.root().join("src/FmtTest.sol")).unwrap(),
        FORMATTED,
    );
});

// Test that fmt can format from stdin
forgetest!(fmt_stdin, |_prj, cmd| {
    cmd.args(["fmt", "-", "--raw"]);
    cmd.stdin(UNFORMATTED.as_bytes());
    cmd.assert_success().stdout_eq(FORMATTED);

    // stdin with `--raw` returns formatted code
    cmd.stdin(FORMATTED.as_bytes());
    cmd.assert_success().stdout_eq(FORMATTED);

    // stdin with `--check` and without `--raw`returns diff
    cmd.forge_fuse().args(["fmt", "-", "--check"]);
    cmd.assert_success().stdout_eq("");
});

forgetest_init!(fmt_check_mode, |prj, cmd| {
    // Run fmt --check on a well-formatted file
    prj.add_raw_source("Test.sol", FORMATTED);
    cmd.arg("fmt").arg("--check").arg("src/Test.sol");
    cmd.assert_success().stderr_eq("").stdout_eq("");

    // Run fmt --check on a mal-formatted file
    prj.add_raw_source("Test2.sol", UNFORMATTED);
    cmd.forge_fuse().arg("fmt").arg("--check").arg("src/Test2.sol");
    cmd.assert_failure();
});

forgetest!(fmt_check_mode_stdin, |_prj, cmd| {
    // Run fmt --check with well-formatted stdin input
    cmd.arg("fmt").arg("-").arg("--check");
    cmd.stdin(FORMATTED.as_bytes());
    cmd.assert_success().stderr_eq("").stdout_eq("");

    // Run fmt --check with mal-formatted stdin input
    cmd.stdin(UNFORMATTED.as_bytes());
    cmd.assert_failure().stderr_eq("").stdout_eq(str![[r#"
Diff in stdin:
1   1    | // SPDX-License-Identifier: MIT
2        |-pragma         solidity  =0.8.30    ;
    2    |+pragma solidity =0.8.30;
...
4        |-contract  Test  {
5        |-    uint256    public    value ;
6        |-    function   setValue ( uint256   _value )   public   {
7        |-        value   =   _value ;
    4    |+contract Test {
    5    |+    uint256 public value;
...
    7    |+    function setValue(uint256 _value) public {
    8    |+        value = _value;
8   9    |     }
9        |-}
    10   |+}

"#]]);
});

// Test that original is returned if read from stdin and no diff.
// <https://github.com/foundry-rs/foundry/issues/11871>
forgetest!(fmt_stdin_original, |_prj, cmd| {
    cmd.args(["fmt", "-", "--raw"]);

    cmd.stdin(FORMATTED.as_bytes());
    cmd.assert_success().stdout_eq(FORMATTED.as_bytes());
});

// Test that fmt can format a simple contract file
forgetest_init!(fmt_file_config_parms_first, |prj, cmd| {
    prj.create_file(
        "foundry.toml",
        r#"
[fmt]
multiline_func_header = 'params_first'
"#,
    );
    prj.add_raw_source("FmtTest.sol", FORMATTED);
    cmd.forge_fuse().args(["fmt", "--check"]).arg("src/FmtTest.sol");
    cmd.assert_failure().stdout_eq(str![[r#"
Diff in src/FmtTest.sol:
...
7        |-    function setValue(uint256 _value) public {
    7    |+    function setValue(
    8    |+        uint256 _value
    9    |+    ) public {
...

"#]]);
});

// https://github.com/foundry-rs/foundry/issues/12000
forgetest_init!(fmt_only_cmnts_file, |prj, cmd| {
    // Only line breaks
    prj.add_raw_source("FmtTest.sol", "\n\n");

    cmd.forge_fuse().args(["fmt", "src/FmtTest.sol"]);
    cmd.assert_success();
    assert_data_eq!(std::fs::read_to_string(prj.root().join("src/FmtTest.sol")).unwrap(), "",);
    cmd.forge_fuse().args(["fmt", "--check", "src/FmtTest.sol"]);
    cmd.assert_success();

    // Only cmnts
    prj.add_raw_source("FmtTest.sol", "\n\n// this is a cmnt");

    cmd.forge_fuse().args(["fmt", "src/FmtTest.sol"]);
    cmd.assert_success();
    assert_data_eq!(
        std::fs::read_to_string(prj.root().join("src/FmtTest.sol")).unwrap(),
        "// this is a cmnt\n",
    );
    cmd.forge_fuse().args(["fmt", "--check", "src/FmtTest.sol"]);
    cmd.assert_success();
});
