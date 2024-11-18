// Ensure we can run basic counter tests with EOF support.
#[cfg(target_os = "linux")]
forgetest_init!(test_eof_flag, |prj, cmd| {
    cmd.forge_fuse().args(["test", "--eof"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
Warning (3805): This is a pre-release compiler version, please do not use it in production.


Ran 2 tests for test/Counter.t.sol:CounterTest
[PASS] testFuzz_SetNumber(uint256) (runs: 256, [AVG_GAS])
[PASS] test_Increment() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});
