//! Integration tests for `forge fmt` command

use foundry_test_utils::{forgetest, snapbox::IntoData};

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

#[forgetest]
fn fmt_exclude_libs_in_recursion(prj: _, cmd: _) {
    prj.update_config(|config| config.fmt.ignore = vec!["src/ignore/".to_string()]);

    prj.add_lib("SomeLib.sol", UNFORMATTED);
    prj.add_raw_source("ignore/IgnoredContract.sol", UNFORMATTED);
    cmd.args(["fmt", ".", "--check"]);
    cmd.assert_success();

    cmd.forge_fuse().args(["fmt", "lib/SomeLib.sol", "--check"]);
    cmd.assert_failure();
}

// Test that fmt can format a simple contract file
#[forgetest]
fn fmt_file(prj: _, cmd: _) {
    prj.add_raw_source("FmtTest.sol", UNFORMATTED);
    cmd.arg("fmt").arg("src/FmtTest.sol");
    cmd.assert_success().stdout_eq(str![""]).stderr_eq(str![[r#"
Formatted [..]/src/FmtTest.sol

"#]]);
    assert_data_eq!(
        std::fs::read_to_string(prj.root().join("src/FmtTest.sol")).unwrap(),
        FORMATTED,
    );
}

// Test that fmt can format from stdin
#[forgetest]
fn fmt_stdin(cmd: _) {
    cmd.args(["fmt", "-", "--raw"]);
    cmd.stdin(UNFORMATTED.as_bytes());
    cmd.assert_success().stdout_eq(FORMATTED);

    // Already formatted stdin is returned unchanged.
    // <https://github.com/foundry-rs/foundry/issues/11871>
    cmd.stdin(FORMATTED.as_bytes());
    cmd.assert_success().stdout_eq(FORMATTED.as_bytes());

    // stdin with `--check` and without `--raw`returns diff
    cmd.forge_fuse().args(["fmt", "-", "--check"]);
    cmd.assert_success().stdout_eq("");
}

#[forgetest]
fn fmt_check_mode(prj: _, cmd: _) {
    // Run fmt --check on a well-formatted file
    prj.add_raw_source("Test.sol", FORMATTED);
    cmd.arg("fmt").arg("--check").arg("src/Test.sol");
    cmd.assert_success().stderr_eq("").stdout_eq("");

    // Run fmt --check on a mal-formatted file
    prj.add_raw_source("Test2.sol", UNFORMATTED);
    cmd.forge_fuse().arg("fmt").arg("--check").arg("src/Test2.sol");
    cmd.assert_failure();
}

#[forgetest]
fn fmt_check_mode_stdin(cmd: _) {
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
}

// Test that fmt can format a simple contract file
#[forgetest]
fn fmt_file_config_parms_first(prj: _, cmd: _) {
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
}

// <https://github.com/foundry-rs/foundry/issues/5686>
#[forgetest]
fn fmt_uses_nearest_config(prj: _, cmd: _) {
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
}

#[forgetest]
fn fmt_without_nearest_uses_invocation_config(prj: _, cmd: _) {
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
}

#[forgetest]
fn fmt_nearest_config_emits_nested_warnings(prj: _, cmd: _) {
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
}

#[forgetest]
fn fmt_nearest_config_rejects_config_env(prj: _, cmd: _) {
    prj.create_file("src/Test.sol", "contract Test {}\n");
    cmd.env("FOUNDRY_CONFIG", "custom.toml");
    cmd.args(["fmt", "--nearest", "src/Test.sol"]).assert_failure().stderr_eq(str![[r#"
Error: `--nearest` cannot be used when `FOUNDRY_CONFIG` is set

"#]]);
}

// https://github.com/foundry-rs/foundry/issues/12000
#[forgetest]
fn fmt_only_cmnts_file(prj: _, cmd: _) {
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
}

// <https://github.com/foundry-rs/foundry/issues/16268>
#[forgetest]
fn fmt_keeps_disable_directive_in_every_file(prj: _, cmd: _) {
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
}

// Symlinks found while walking directories must not make `forge fmt` write outside of the project.
#[cfg(unix)]
#[forgetest]
fn fmt_skips_symlinks_outside_project(prj: _, cmd: _) {
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("Outside.sol");
    let outside_dir_file = outside.path().join("dir/Dir.sol");
    std::fs::create_dir(outside.path().join("dir")).unwrap();
    std::fs::write(&outside_file, UNFORMATTED).unwrap();
    std::fs::write(&outside_dir_file, UNFORMATTED).unwrap();

    let own = prj.add_raw_source("Own.sol", UNFORMATTED);
    std::os::unix::fs::symlink(&outside_file, prj.root().join("src/Link.sol")).unwrap();
    std::fs::create_dir_all(prj.root().join("test")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("dir"), prj.root().join("test/linked")).unwrap();

    cmd.arg("fmt").assert_success().stderr_eq(str![[r#"
Warning: Skipping [..]/src/Link.sol: it resolves outside of the project root.
HINT: Pass the path explicitly to format it: `forge fmt <paths>`
Warning: Skipping [..]/test/linked/Dir.sol: it resolves outside of the project root.
HINT: Pass the path explicitly to format it: `forge fmt <paths>`
Formatted [..]/src/Own.sol

"#]]);
    assert_data_eq!(std::fs::read_to_string(own).unwrap(), FORMATTED);
    assert_data_eq!(std::fs::read_to_string(&outside_file).unwrap(), UNFORMATTED);
    assert_data_eq!(std::fs::read_to_string(&outside_dir_file).unwrap(), UNFORMATTED);

    // Walking an explicit project directory applies the same rule.
    cmd.forge_fuse().args(["fmt", "test"]).assert_success().stderr_eq(str![[r#"
Warning: Skipping test/linked/Dir.sol: it resolves outside of the project root.
HINT: Pass the path explicitly to format it: `forge fmt <paths>`
Warning: Nothing to format.
HINT: If you are working outside of the project, try providing paths to your source files: `forge fmt <paths>`

"#]]);
    assert_data_eq!(std::fs::read_to_string(&outside_dir_file).unwrap(), UNFORMATTED);

    // Explicit paths remain an opt-in.
    cmd.forge_fuse().args(["fmt", "src/Link.sol", "test/linked"]).assert_success();
    assert_data_eq!(std::fs::read_to_string(&outside_file).unwrap(), FORMATTED);
    assert_data_eq!(std::fs::read_to_string(&outside_dir_file).unwrap(), FORMATTED);
}

#[forgetest]
fn fmt_external_configured_directories(prj: _, cmd: _) {
    let outside = tempfile::tempdir().unwrap();
    for name in ["src", "test", "script"] {
        let dir = outside.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("External.sol"), UNFORMATTED).unwrap();
    }
    prj.update_config(|config| {
        config.src = outside.path().join("src");
        config.test = outside.path().join("test");
        config.script = outside.path().join("script");
    });

    cmd.args(["fmt", "--check"]).assert_failure().stderr_eq("");
    for name in ["src", "test", "script"] {
        assert_data_eq!(
            std::fs::read_to_string(outside.path().join(name).join("External.sol")).unwrap(),
            UNFORMATTED,
        );
    }

    cmd.forge_fuse().arg("fmt").assert_success().stdout_eq("").stderr_eq(
        str![[r#"
Formatted [..]/src/External.sol
Formatted [..]/test/External.sol
Formatted [..]/script/External.sol

"#]]
        .unordered(),
    );
    for name in ["src", "test", "script"] {
        assert_data_eq!(
            std::fs::read_to_string(outside.path().join(name).join("External.sol")).unwrap(),
            FORMATTED,
        );
    }
    cmd.forge_fuse().args(["fmt", "--check"]).assert_success().stdout_eq("").stderr_eq("");
}

#[forgetest]
fn fmt_configured_files_keep_default_filters(prj: _, cmd: _) {
    for dir in ["lib", "src"] {
        let files =
            ["Vendor.sol", "Vendor.t.sol", "Vendor.s.sol"].map(|name| format!("{dir}/{name}"));
        for file in &files {
            prj.create_file(file, UNFORMATTED);
        }
        prj.update_config(|config| {
            config.src = files[0].clone().into();
            config.test = files[1].clone().into();
            config.script = files[2].clone().into();
            config.fmt.ignore = vec!["src".to_string()];
        });

        for args in [vec!["fmt"], vec!["fmt", "--check"]] {
            cmd.forge_fuse().args(args).assert_success().stdout_eq("").stderr_eq(str![[r#"
Warning: Nothing to format.
HINT: If you are working outside of the project, try providing paths to your source files: `forge fmt <paths>`

"#]]);
            for file in &files {
                assert_data_eq!(
                    std::fs::read_to_string(prj.root().join(file)).unwrap(),
                    UNFORMATTED
                );
            }
        }

        // Explicit CLI files still opt in to libraries and ignored directories.
        cmd.forge_fuse().arg("fmt").args(&files).assert_success().stdout_eq("").stderr_eq(
            str![[r#"
Formatted [..]/Vendor.sol
Formatted [..]/Vendor.t.sol
Formatted [..]/Vendor.s.sol

"#]]
            .unordered(),
        );
        for file in &files {
            assert_data_eq!(std::fs::read_to_string(prj.root().join(file)).unwrap(), FORMATTED);
        }
    }
}

#[forgetest]
fn fmt_configured_files(prj: _, cmd: _) {
    let files = ["Source.sol", "Test.sol", "Script.sol"];
    for file in files {
        prj.create_file(file, UNFORMATTED);
    }
    prj.update_config(|config| {
        config.src = files[0].into();
        config.test = files[1].into();
        config.script = files[2].into();
    });

    cmd.arg("fmt").assert_success().stdout_eq("").stderr_eq(
        str![[r#"
Formatted [..]/Source.sol
Formatted [..]/Test.sol
Formatted [..]/Script.sol

"#]]
        .unordered(),
    );
    for file in files {
        assert_data_eq!(std::fs::read_to_string(prj.root().join(file)).unwrap(), FORMATTED);
    }
    cmd.forge_fuse().args(["fmt", "--check"]).assert_success().stdout_eq("").stderr_eq("");
}
