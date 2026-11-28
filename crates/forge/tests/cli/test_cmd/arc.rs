#[forgetest]
fn arc_enforces_native_transfer_rules(prj: _, cmd: _) {
    prj.add_test("ArcEvm.t.sol", include_str!("../../fixtures/ArcEvm.t.sol"));

    cmd.args(["test", "--network", "arc", "--match-test", "test_(value|precompile)"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 3 tests for test/ArcEvm.t.sol:ArcEvmTest
[PASS] test_precompile_accounts_hold_code() ([GAS])
[PASS] test_value_to_blocklisted_address_reverts() ([GAS])
[PASS] test_value_to_zero_address_reverts() ([GAS])
Suite result: ok. 3 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest]
fn arc_defaults_to_zero8(prj: _, cmd: _) {
    prj.add_test("ArcEvm.t.sol", include_str!("../../fixtures/ArcEvm.t.sol"));

    cmd.args(["test", "--network", "arc", "--match-test", "test_call_from_rejects"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ArcEvm.t.sol:ArcEvmTest
[PASS] test_call_from_rejects_unauthorized_callers() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest]
fn arc_hardfork_selects_arc_network(prj: _, cmd: _) {
    prj.add_test("ArcEvm.t.sol", include_str!("../../fixtures/ArcEvm.t.sol"));

    cmd.args(["test", "--hardfork", "arc:Zero6", "--match-test", "test_call_from_is_absent"])
        .assert_success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/ArcEvm.t.sol:ArcEvmTest
[PASS] test_call_from_is_absent_before_zero7() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);
}
