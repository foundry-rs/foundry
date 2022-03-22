//! Contains various tests for checking cast commands
use foundry_cli_test_utils::{
    casttest,
    util::{TestCommand, TestProject},
};

// tests that the `cast run` command works correctly
casttest!(run_bytecode, |_: TestProject, mut cmd: TestCommand| {
    // Create executable bytecode
    let raw_bytecode = "0x604260005260206000F3".to_string();
    let raw_calldata =
        "0x70a08231000000000000000000000000b4c79dab8f259c7aee6e5b2aa729821864227e84".to_string();

    // Call `cast run`
    cmd.arg("run").arg(raw_bytecode).arg(raw_calldata);
    let output = cmd.stdout_lossy();

    // Expect successful bytecode execution
    assert!(output.contains("[SUCCESS]"));
    assert!(output.contains("Gas consumed:"));
    assert!(output.contains("No Output"));

    // Create reverting bytecode
    let reverting_bytecode = "0x610101610102016000526001601ff3".to_string();
    let revert_calldata =
        "0x70a08231000000000000000000000000b4c79dab8f259c7aee6e5b2aa729821864227e84".to_string();

    // Call `cast run`
    cmd.cfuse().arg("run").arg(reverting_bytecode).arg(revert_calldata);
    let revert_output = cmd.stdout_lossy();

    // Expect bytecode execution to revert
    assert!(revert_output.contains("[REVERT]"));
    assert!(revert_output.contains("Gas consumed:"));

    // On revert, we don't have an output
    assert!(!revert_output.contains("Output"));
});

// tests that `cast run` works without calldata
casttest!(run_bytecode_absent_calldata, |_: TestProject, mut cmd: TestCommand| {
    // Create executable bytecode
    let raw_bytecode = "0x604260005260206000F3".to_string();

    // Call `cast run`
    cmd.arg("run").arg(raw_bytecode);
    let output = cmd.stdout_lossy();

    // Expect successful bytecode execution
    assert!(output.contains("[SUCCESS]"));
    assert!(output.contains("Gas consumed:"));
    assert!(output.contains("No Output"));

    // Create reverting bytecode
    let reverting_bytecode = "0x610101610102016000526001601ff3".to_string();

    // Call `cast run`
    cmd.cfuse().arg("run").arg(reverting_bytecode);
    let revert_output = cmd.stdout_lossy();

    // Expect bytecode execution to revert
    assert!(revert_output.contains("[REVERT]"));
    assert!(revert_output.contains("Gas consumed:"));

    // On revert, we don't have an output
    assert!(!revert_output.contains("Output"));
});
