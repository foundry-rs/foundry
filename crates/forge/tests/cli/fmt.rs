//! Integration tests for `forge fmt` command

use foundry_test_utils::{forgetest, forgetest_init};

const UNFORMATTED: &str = r#"// SPDX-License-Identifier: MIT
pragma         solidity  =0.8.33    ;

contract  Test  {
    uint256    public    value ;
    function   setValue ( uint256   _value )   public   {
        value   =   _value ;
    }
}"#;

const FORMATTED: &str = r#"// SPDX-License-Identifier: MIT
pragma solidity =0.8.33;

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
    cmd.assert_success().stdout_eq(str![""]).stderr_eq(str![[r#"
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
2        |-pragma         solidity  =0.8.33    ;
    2    |+pragma solidity =0.8.33;
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

// <https://github.com/foundry-rs/foundry/issues/5686>
forgetest!(fmt_uses_nearest_config, |prj, cmd| {
    let source = r#"contract Test {
    function test(uint256 a, uint256 b) public returns (uint256) { return a + b; }
}
"#;
    let first = prj.create_file("first/src/Test.sol", source);
    let second = prj.create_file("second/src/Test.sol", source);
    let ignored = prj.create_file("first/src/Ignored.sol", source);
    prj.create_file(
        "foundry.toml",
        r#"[fmt]
ignore = ["first/src/Test.sol", "["]
"#,
    );
    prj.create_file(
        "first/foundry.toml",
        r#"[fmt]
multiline_func_header = "params_first"
tab_width = 2
ignore = ["src/Ignored.sol"]
"#,
    );
    prj.create_file(
        "second/foundry.toml",
        r#"[fmt]
multiline_func_header = "attributes_first"
tab_width = 6
"#,
    );

    let output = cmd.args(["fmt", "--nearest", "--check", "."]).assert_failure();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).replace('\\', "/");
    assert!(stdout.contains("Diff in first/src/Test.sol"), "{stdout}");
    assert!(stdout.contains("|+  function test("), "{stdout}");
    assert!(stdout.contains("Diff in second/src/Test.sol"), "{stdout}");
    assert!(stdout.contains("|+      function test(uint256 a, uint256 b)"), "{stdout}");

    cmd.forge_fuse().args(["fmt", "--nearest", "."]).assert_success();

    assert!(std::fs::read_to_string(first).unwrap().contains("  function test(\n"));
    assert!(
        std::fs::read_to_string(second)
            .unwrap()
            .contains("      function test(uint256 a, uint256 b)")
    );
    assert_eq!(std::fs::read_to_string(ignored).unwrap(), source);
});

forgetest!(fmt_without_nearest_uses_invocation_config, |prj, cmd| {
    let file = prj.create_file(
        "nested/src/Test.sol",
        r#"contract Test {
    function test(uint256 a, uint256 b) public returns (uint256) { return a + b; }
}
"#,
    );
    prj.create_file(
        "foundry.toml",
        r#"[fmt]
multiline_func_header = "params_first"
tab_width = 4
"#,
    );
    prj.create_file(
        "nested/foundry.toml",
        r#"[fmt]
multiline_func_header = "attributes_first"
tab_width = 2
"#,
    );

    cmd.args(["fmt", "nested/src/Test.sol"]).assert_success();

    assert!(std::fs::read_to_string(file).unwrap().contains("    function test(\n"));
});

forgetest!(fmt_nearest_config_emits_nested_warnings, |prj, cmd| {
    prj.create_file("nested/src/Test.sol", "contract Test {}\n");
    prj.create_file(
        "nested/foundry.toml",
        r#"[default]
src = "src"
"#,
    );

    cmd.args(["fmt", "--nearest", "nested/src/Test.sol"])
        .assert_success()
        .stderr_eq(str![[r#"
Warning: Found unknown config section in nested/foundry.toml: [default]
This notation for profiles has been deprecated and may result in the profile not being registered in future versions.
Please use [profile.default] instead or run `forge config --fix`.

"#]]);
});

forgetest!(fmt_nearest_config_rejects_config_env, |prj, cmd| {
    prj.create_file("src/Test.sol", "contract Test {}\n");
    cmd.env("FOUNDRY_CONFIG", "custom.toml");
    cmd.args(["fmt", "--nearest", "src/Test.sol"]).assert_failure().stderr_eq(str![[r#"
Error: `--nearest` cannot be used when `FOUNDRY_CONFIG` is set

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

// <https://github.com/foundry-rs/foundry/issues/16268>
forgetest_init!(fmt_keeps_disable_directive_in_every_file, |prj, cmd| {
    const NAMES: [&str; 4] = ["A", "B", "C", "D"];
    const SOURCE: &str = "// forgefmt: disable-next-line\ncontract  Disabled {}\n";

    for name in NAMES {
        prj.add_raw_source(&format!("Fmt{name}.sol"), SOURCE);
    }

    cmd.args(["fmt", "src"]).assert_success();

    // Only the first file in the source map used to keep its directive.
    for name in NAMES {
        assert_data_eq!(
            std::fs::read_to_string(prj.root().join(format!("src/Fmt{name}.sol"))).unwrap(),
            SOURCE,
        );
    }
});
