//! Campaign execution-boundary parity checks shared by REVM and evm2.

use alloy_primitives::{Address, B256, Bytes, U256, hex};
use anvil::{NodeConfig, spawn};
use foundry_test_utils::{TestCommand, TestProject, str};

const CASE_ISOLATION: &str =
    include_str!("../../../../../testdata/fixtures/FuzzCaseIsolation.t.sol");

fn setup_case_isolation(prj: &mut TestProject, cmd: &mut TestCommand) {
    prj.add_test("FuzzCaseIsolation.t.sol", CASE_ISOLATION);
    cmd.args(["test", "--fuzz-runs", "16", "--fuzz-seed", "1"]);
}

forgetest!(campaign_case_isolation, |prj, cmd| {
    setup_case_isolation(&mut prj, &mut cmd);
    cmd.args(["--match-contract", "^LocalFuzzCaseIsolationTest$"]).assert_success().stdout_eq(
        str![[r#"
...
Ran 2 tests for test/FuzzCaseIsolation.t.sol:LocalFuzzCaseIsolationTest
[PASS] testFuzzAcceptedCaseIsolation(uint256) (runs: 16, [AVG_GAS])
[PASS] testFuzzRejectedCaseIsolation(bool) (runs: 16, [AVG_GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
...
"#]],
    );
});

forgetest_async!(campaign_fork_case_isolation, |prj, cmd| {
    let (api, handle) = spawn(NodeConfig::test()).await;
    let target = Address::from_word(B256::from(U256::from(0x10000)));
    api.anvil_set_code(target, Bytes::from_static(&hex!("60005460005260206000f3"))).await.unwrap();
    api.anvil_set_storage_at(target, U256::ZERO, B256::from(U256::from(41))).await.unwrap();
    api.anvil_mine(Some(U256::ONE), None).await.unwrap();

    setup_case_isolation(&mut prj, &mut cmd);
    cmd.args([
        "--match-contract",
        "^ForkFuzzCaseIsolationTest$",
        "--fork-url",
        &handle.http_endpoint(),
        "--fork-block-number",
        "1",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
...
Ran 2 tests for test/FuzzCaseIsolation.t.sol:ForkFuzzCaseIsolationTest
[PASS] testFuzzAcceptedCaseIsolation(uint256) (runs: 16, [AVG_GAS])
[PASS] testFuzzRejectedCaseIsolation(bool) (runs: 16, [AVG_GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)
...
"#]]);
});
