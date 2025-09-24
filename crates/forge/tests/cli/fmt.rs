//! Integration tests for `forge fmt` command

use foundry_test_utils::{forgetest, forgetest_init};
use std::{fs, io::Write};

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

// Test that fmt can format a simple contract file
forgetest_init!(fmt_file, |prj, cmd| {
    prj.add_source("FmtTest.sol", UNFORMATTED);
    cmd.arg("fmt").arg("src/FmtTest.sol");
    cmd.assert_success();

    // Check that the file was formatted
    let formatted = fs::read_to_string(prj.root().join("src/FmtTest.sol")).unwrap();
    assert!(formatted.contains(FORMATTED));
});

// Test that fmt can format from stdin
forgetest!(fmt_stdin, |_prj, cmd| {
    cmd.args(["fmt", "-", "--raw"]);
    cmd.stdin(move |mut stdin| {
        stdin.write_all(UNFORMATTED.as_bytes()).unwrap();
    });

    // Check the output contains formatted code
    cmd.assert_success().stdout_eq(FORMATTED);
});

forgetest_init!(fmt_check_mode, |prj, cmd| {
    // Run fmt --check on a well-formatted file
    prj.add_source("Test.sol", FORMATTED);
    cmd.arg("fmt").arg("--check").arg("src/Test.sol");
    cmd.assert_success();

    // Run fmt --check on a mal-formatted file
    prj.add_source("Test2.sol", UNFORMATTED);
    let mut cmd2 = prj.forge_command();
    cmd2.arg("fmt").arg("--check").arg("src/Test2.sol");
    cmd2.assert_failure();
});

forgetest!(fmt_check_mode_stdin, |_prj, cmd| {
    // Run fmt --check with well-formatted stdin input
    cmd.arg("fmt").arg("-").arg("--check");
    cmd.stdin(move |mut stdin| {
        stdin.write_all(FORMATTED.as_bytes()).unwrap();
    });
    cmd.assert_success();

    // Run fmt --check with mal-formatted stdin input
    cmd.arg("fmt").arg("-").arg("--check");
    cmd.stdin(move |mut stdin| {
        stdin.write_all(UNFORMATTED.as_bytes()).unwrap();
    });
    cmd.assert_failure();
});
