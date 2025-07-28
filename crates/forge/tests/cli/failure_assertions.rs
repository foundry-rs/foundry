// Tests in which we want to assert failures.

forgetest!(test_fail_deprecation, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "DeprecationTestFail.t.sol",
        r#"
    import "./test.sol";
    contract DeprecationTestFail is DSTest {
        function testFail_deprecated() public {
            revert("deprecated");
        }

        function testFail_deprecated2() public {
            revert("deprecated2");
        }
    }
    "#,
    )
    .unwrap();

    cmd.forge_fuse().args(["test", "--mc", "DeprecationTestFail"]).assert_failure().stdout_eq(
        r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL: `testFail*` has been removed. Consider changing to test_Revert[If|When]_Condition and expecting a revert] Found 2 instances: testFail_deprecated, testFail_deprecated2 ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#,
    );
});

forgetest!(expect_revert_tests_should_fail, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    let expect_revert_failure_tests = include_str!("../fixtures/ExpectRevertFailures.t.sol");

    prj.add_source("ExpectRevertFailures.sol", expect_revert_failure_tests).unwrap();

    cmd.forge_fuse()
        .args(["test", "--mc", "ExpectRevertFailureTest"])
        .assert_failure()
        .stdout_eq(
            r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL: next call did not revert as expected] testShouldFailExpectRevertAnyRevertDidNotRevert() ([GAS])
[FAIL: next call did not revert as expected] testShouldFailExpectRevertDangling() ([GAS])
[FAIL: next call did not revert as expected] testShouldFailExpectRevertDidNotRevert() ([GAS])
[FAIL: Error != expected error: but reverts with this message != should revert with this message] testShouldFailExpectRevertErrorDoesNotMatch() ([GAS])
[FAIL: next call did not revert as expected] testShouldFailRevertNotOnImmediateNextCall() ([GAS])
[FAIL: some message] testShouldFailexpectCheatcodeRevertForCreate() ([GAS])
[FAIL: revert] testShouldFailexpectCheatcodeRevertForExtCall() ([GAS])
Suite result: FAILED. 0 passed; 7 failed; 0 skipped; [ELAPSED]
...
"#,
        );

    cmd.forge_fuse()
        .args(["test", "--mc", "ExpectRevertWithReverterFailureTest"])
        .assert_failure()
        .stdout_eq(
            r#"No files changed, compilation skipped
...
[FAIL: next call did not revert as expected] testShouldFailExpectRevertsNotOnImmediateNextCall() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#,
        );

    cmd.forge_fuse()
        .args(["test", "--mc", "ExpectRevertCountFailureTest"])
        .assert_failure()
        .stdout_eq(
            r#"No files changed, compilation skipped
...
[FAIL: call reverted with 'my cool error' when it was expected not to revert] testShouldFailIfExpectRevertWrongString() ([GAS])
[FAIL: call reverted when it was expected not to revert] testShouldFailNoRevert() ([GAS])
[FAIL: expected 0 reverts with reason: revert, but got one] testShouldFailNoRevertSpecific() ([GAS])
[FAIL: next call did not revert as expected] testShouldFailRevertCountAny() ([GAS])
[FAIL: Error != expected error: wrong revert != called a function and then reverted] testShouldFailRevertCountCallsThenReverts() ([GAS])
[FAIL: Error != expected error: second-revert != revert] testShouldFailRevertCountSpecific() ([GAS])
Suite result: FAILED. 0 passed; 6 failed; 0 skipped; [ELAPSED]
...
"#,
        );

    cmd.forge_fuse()
        .args(["test", "--mc", "ExpectRevertCountWithReverterFailures"])
        .assert_failure()
        .stdout_eq(r#"No files changed, compilation skipped
...
[FAIL: call reverted with 'revert' from 0x2e234DAe75C793f67A35089C9d99245E1C58470b, but expected 0 reverts from 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f] testNoRevertWithWrongReverter() ([GAS])
[FAIL: call reverted with 'revert2' from 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f, but expected 0 reverts with reason 'revert' from 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f] testNoReverterCountWithData() ([GAS])
[FAIL: expected 0 reverts from address: 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f, but got one] testShouldFailNoRevertWithReverter() ([GAS])
[FAIL: Reverter != expected reverter: 0x2e234DAe75C793f67A35089C9d99245E1C58470b != 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f] testShouldFailRevertCountWithReverter() ([GAS])
[FAIL: Error != expected error: wrong revert != revert] testShouldFailReverterCountWithWrongData() ([GAS])
[FAIL: Reverter != expected reverter: 0x2e234DAe75C793f67A35089C9d99245E1C58470b != 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f] testShouldFailWrongReverterCountWithData() ([GAS])
Suite result: FAILED. 0 passed; 6 failed; 0 skipped; [ELAPSED]
...
"#);
});

forgetest!(expect_call_tests_should_fail, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();

    let expect_call_failure_tests = include_str!("../fixtures/ExpectCallFailures.t.sol");

    prj.add_source("ExpectCallFailures.sol", expect_call_failure_tests).unwrap();

    cmd.forge_fuse().args(["test", "--mc", "ExpectCallFailureTest"]).assert_failure().stdout_eq(
        r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0xc290d6910000000000000000000000000000000000000000000000000000000000000002, value 1 to be called 1 time, but was called 0 times] testShouldFailExpectCallValue() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002 to be called 1 time, but was called 0 times] testShouldFailExpectCallWithData() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f7000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000003 to be called 1 time, but was called 0 times] testShouldFailExpectCallWithMoreParameters() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001, value 0, gas 25000 to be called 1 time, but was called 0 times] testShouldFailExpectCallWithNoValueAndWrongGas() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001, value 0, minimum gas 50001 to be called 1 time, but was called 0 times] testShouldFailExpectCallWithNoValueAndWrongMinGas() ([GAS])
[FAIL: next call did not revert as expected] testShouldFailExpectCallWithRevertDisallowed() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x3fc7c698 to be called 1 time, but was called 0 times] testShouldFailExpectInnerCall() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002 to be called 3 times, but was called 2 times] testShouldFailExpectMultipleCallsWithDataAdditive() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f7 to be called 1 time, but was called 0 times] testShouldFailExpectSelectorCall() ([GAS])
Suite result: FAILED. 0 passed; 9 failed; 0 skipped; [ELAPSED]
...
"#,
    );

    cmd.forge_fuse()
        .args(["test", "--mc", "ExpectCallCountFailureTest"])
        .assert_failure()
        .stdout_eq(
            r#"No files changed, compilation skipped
...
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0xc290d6910000000000000000000000000000000000000000000000000000000000000002, value 1 to be called 1 time, but was called 0 times] testShouldFailExpectCallCountValue() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001, value 0, gas 25000 to be called 2 times, but was called 0 times] testShouldFailExpectCallCountWithNoValueAndWrongGas() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000001, value 0, minimum gas 50001 to be called 1 time, but was called 0 times] testShouldFailExpectCallCountWithNoValueAndWrongMinGas() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x771602f700000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000002 to be called 2 times, but was called 1 time] testShouldFailExpectCallCountWithWrongCount() ([GAS])
[FAIL: expected call to 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f with data 0x3fc7c698 to be called 1 time, but was called 0 times] testShouldFailExpectCountInnerCall() ([GAS])
Suite result: FAILED. 0 passed; 5 failed; 0 skipped; [ELAPSED]
...
"#,
        );

    cmd.forge_fuse()
        .args(["test", "--mc", "ExpectCallMixedFailureTest"])
        .assert_failure()
        .stdout_eq(
            r#"No files changed, compilation skipped
...
[FAIL: vm.expectCall: counted expected calls can only bet set once] testShouldFailOverrideCountWithCount() ([GAS])
[FAIL: vm.expectCall: cannot overwrite a counted expectCall with a non-counted expectCall] testShouldFailOverrideCountWithNoCount() ([GAS])
[FAIL: vm.expectCall: counted expected calls can only bet set once] testShouldFailOverrideNoCountWithCount() ([GAS])
Suite result: FAILED. 0 passed; 3 failed; 0 skipped; [ELAPSED]
...
"#,
        );
});

forgetest!(expect_create_tests_should_fail, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();

    let expect_create_failures = include_str!("../fixtures/ExpectCreateFailures.t.sol");

    prj.add_source("ExpectCreateFailures.t.sol", expect_create_failures).unwrap();

    cmd.forge_fuse().args(["test", "--mc", "ExpectCreateFailureTest"]).assert_failure().stdout_eq(str![[r#"
...
[FAIL: expected CREATE call by address 0x7fa9385be102ac3eac297483dd6233d62b3e1496 for bytecode [..] but not found] testShouldFailExpectCreate() ([GAS])
[FAIL: expected CREATE2 call by address 0x7fa9385be102ac3eac297483dd6233d62b3e1496 for bytecode [..] but not found] testShouldFailExpectCreate2() ([GAS])
[FAIL: expected CREATE2 call by address 0x7fa9385be102ac3eac297483dd6233d62b3e1496 for bytecode [..] but not found] testShouldFailExpectCreate2WrongBytecode() ([GAS])
[FAIL: expected CREATE2 call by address 0x0000000000000000000000000000000000000000 for bytecode [..] but not found] testShouldFailExpectCreate2WrongDeployer() ([GAS])
[FAIL: expected CREATE2 call by address 0x7fa9385be102ac3eac297483dd6233d62b3e1496 for bytecode [..] but not found] testShouldFailExpectCreate2WrongScheme() ([GAS])
[FAIL: expected CREATE call by address 0x7fa9385be102ac3eac297483dd6233d62b3e1496 for bytecode [..] but not found] testShouldFailExpectCreateWrongBytecode() ([GAS])
[FAIL: expected CREATE call by address 0x0000000000000000000000000000000000000000 for bytecode [..] but not found] testShouldFailExpectCreateWrongDeployer() ([GAS])
[FAIL: expected CREATE call by address 0x7fa9385be102ac3eac297483dd6233d62b3e1496 for bytecode [..] but not found] testShouldFailExpectCreateWrongScheme() ([GAS])
Suite result: FAILED. 0 passed; 8 failed; 0 skipped; [ELAPSED]
...

"#]]);
});

forgetest!(expect_emit_tests_should_fail, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();

    let expect_emit_failure_tests = include_str!("../fixtures/ExpectEmitFailures.t.sol");

    prj.add_source("ExpectEmitFailures.sol", expect_emit_failure_tests).unwrap();

    cmd.forge_fuse().arg("build").assert_success();

    cmd.forge_fuse().args(["test", "--mc", "ExpectEmitFailureTest"]).assert_failure().stdout_eq(str![[r#"No files changed, compilation skipped
...
[FAIL: E != expected A] testShouldFailCanMatchConsecutiveEvents() ([GAS])
[FAIL: log != expected SomethingElse] testShouldFailDifferentIndexedParameters() ([GAS])
[FAIL: log != expected log] testShouldFailEmitOnlyAppliesToNextCall() ([GAS])
[FAIL: next call did not revert as expected] testShouldFailEmitWindowWithRevertDisallowed() ([GAS])
[FAIL: E != expected A] testShouldFailEventsOnTwoCalls() ([GAS])
[FAIL: Something param mismatch at [..]: expected=[..], got=[..]; counterexample: calldata=[..] args=[..]] testShouldFailExpectEmit(bool,bool,bool,bool,uint128,uint128,uint128,uint128) (runs: 0, [AVG_GAS])
[FAIL: log emitter mismatch: expected=[..], got=[..]] testShouldFailExpectEmitAddress() ([GAS])
[FAIL: log emitter mismatch: expected=[..], got=[..]] testShouldFailExpectEmitAddressWithArgs() ([GAS])
[FAIL: Something != expected SomethingElse] testShouldFailExpectEmitCanMatchWithoutExactOrder() ([GAS])
[FAIL: expected an emit, but no logs were emitted afterwards. you might have mismatched events or not enough events were emitted] testShouldFailExpectEmitDanglingNoReference() ([GAS])
[FAIL: expected an emit, but no logs were emitted afterwards. you might have mismatched events or not enough events were emitted] testShouldFailExpectEmitDanglingWithReference() ([GAS])
[FAIL: Something param mismatch at [..]: expected=[..], got=[..]; counterexample: calldata=[..] args=[..]] testShouldFailExpectEmitNested(bool,bool,bool,bool,uint128,uint128,uint128,uint128) (runs: 0, [AVG_GAS])
[FAIL: log != expected log] testShouldFailLowLevelWithoutEmit() ([GAS])
[FAIL: log != expected log] testShouldFailMatchRepeatedEventsOutOfOrder() ([GAS])
[FAIL: log != expected log] testShouldFailNoEmitDirectlyOnNextCall() ([GAS])
Suite result: FAILED. 0 passed; 15 failed; 0 skipped; [ELAPSED]
...
"#]]);

    cmd.forge_fuse()
        .args(["test", "--mc", "ExpectEmitCountFailureTest"])
        .assert_failure()
        .stdout_eq(
            r#"No files changed, compilation skipped
...
[FAIL: log != expected log] testShouldFailCountEmitsFromAddress() ([GAS])
[FAIL: log != expected log] testShouldFailCountLessEmits() ([GAS])
[FAIL: log != expected Something] testShouldFailEmitSomethingElse() ([GAS])
[FAIL: log emitted 1 times, expected 0] testShouldFailNoEmit() ([GAS])
[FAIL: log emitted 1 times, expected 0] testShouldFailNoEmitFromAddress() ([GAS])
Suite result: FAILED. 0 passed; 5 failed; 0 skipped; [ELAPSED]
...
"#,
        );
});

forgetest!(expect_emit_params_tests_should_fail, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();

    let expect_emit_failure_src = include_str!("../fixtures/ExpectEmitParamHarness.sol");
    let expect_emit_failure_tests = include_str!("../fixtures/ExpectEmitParamFailures.t.sol");

    prj.add_source("ExpectEmitParamHarness.sol", expect_emit_failure_src).unwrap();
    prj.add_source("ExpectEmitParamFailures.sol", expect_emit_failure_tests).unwrap();

    cmd.forge_fuse().arg("build").assert_success();

    cmd.forge_fuse().args(["test", "--mc", "ExpectEmitParamFailures"]).assert_failure().stdout_eq(
        r#"No files changed, compilation skipped
...
[PASS] testSelectiveChecks() ([GAS])
Suite result: FAILED. 1 passed; 8 failed; 0 skipped; [ELAPSED]
...
[FAIL: anonymous log mismatch at param 0: expected=0x0000000000000000000000000000000000000000000000000000000000000064, got=0x00000000000000000000000000000000000000000000000000000000000003e7] testAnonymousEventMismatch() ([GAS])
[FAIL: ComplexEvent != expected SimpleEvent] testCompletelyDifferentEvent() ([GAS])
[FAIL: SimpleEvent param mismatch at b: expected=200, got=999] testIndexedParamMismatch() ([GAS])
[FAIL: ManyParams param mismatch at a: expected=100, got=111, b: expected=200, got=222, c: expected=300, got=333, d: expected=400, got=444, e: expected=500, got=555] testManyParameterMismatches() ([GAS])
[FAIL: SimpleEvent param mismatch at c: expected=300, got=999] testMixedEventNonIndexedMismatch() ([GAS])
[FAIL: SimpleEvent param mismatch at a: expected=100, got=999, b: expected=200, got=888, c: expected=300, got=777] testMultipleMismatches() ([GAS])
[FAIL: SimpleEvent param mismatch at c: expected=300, got=999] testNonIndexedParamMismatch() ([GAS])
[FAIL: MixedEventNumbering param mismatch at param2: expected=300, got=999] testParameterNumbering() ([GAS])

Encountered a total of 8 failing tests, 1 tests succeeded
...
"#,
    );
});

forgetest!(mem_safety_test_should_fail, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();

    let mem_safety_failure_tests = include_str!("../fixtures/MemSafetyFailures.t.sol");

    prj.add_source("MemSafetyFailures.sol", mem_safety_failure_tests).unwrap();

    cmd.forge_fuse().args(["test", "--mc", "MemSafetyFailureTest"]).assert_failure().stdout_eq(
        r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL: Expected call to fail] testShouldFailExpectSafeMemoryCall() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x60 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_CALL() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x60 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_CALLCODE() ([GAS])
[FAIL: memory write at offset 0xA0 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0xA0]; counterexample: calldata=[..] args=[..]] testShouldFailExpectSafeMemory_CALLDATACOPY(uint256) (runs: 0, [AVG_GAS])
[FAIL: memory write at offset 0x80 of size [..] not allowed; safe range: (0x00, 0x60] U (0x80, 0xA0]] testShouldFailExpectSafeMemory_CODECOPY() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_CREATE() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_CREATE2() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x60 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_DELEGATECALL() ([GAS])
[FAIL: memory write at offset 0xA0 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0xA0]] testShouldFailExpectSafeMemory_EXTCODECOPY() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_LOG0() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_MLOAD() ([GAS])
[FAIL: memory write at offset 0x81 of size 0x01 not allowed; safe range: (0x00, 0x60] U (0x80, 0x81]] testShouldFailExpectSafeMemory_MSTORE8_High() ([GAS])
[FAIL: memory write at offset 0x60 of size 0x01 not allowed; safe range: (0x00, 0x60] U (0x80, 0x81]] testShouldFailExpectSafeMemory_MSTORE8_Low() ([GAS])
[FAIL: memory write at offset 0xA0 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0xA0]] testShouldFailExpectSafeMemory_MSTORE_High() ([GAS])
[FAIL: memory write at offset 0x60 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0xA0]] testShouldFailExpectSafeMemory_MSTORE_Low() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_RETURN() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x60 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_RETURNDATACOPY() ([GAS])
[FAIL: EvmError: Revert] testShouldFailExpectSafeMemory_REVERT() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_SHA3() ([GAS])
[FAIL: memory write at offset 0x100 of size 0x60 not allowed; safe range: (0x00, 0x60] U (0x80, 0x100]] testShouldFailExpectSafeMemory_STATICCALL() ([GAS])
[FAIL: memory write at offset 0xA0 of size 0x20 not allowed; safe range: (0x00, 0x60] U (0x80, 0xA0]] testShouldFailStopExpectSafeMemory() ([GAS])
Suite result: FAILED. 0 passed; 21 failed; 0 skipped; [ELAPSED]
...
"#,
    );
});

forgetest!(ds_style_test_failing, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "DSStyleTest.t.sol",
        r#"
        import "./test.sol";

        contract DSStyleTest is DSTest {
            function testDSTestFailingAssertions() public {
                emit log_string("assertionOne");
                assertEq(uint256(1), uint256(2));
                emit log_string("assertionTwo");
                assertEq(uint256(3), uint256(4));
                emit log_string("done");
            }
        }
        "#,
    )
    .unwrap();

    cmd.forge_fuse().args(["test", "--mc", "DSStyleTest", "-vv"]).assert_failure().stdout_eq(
        r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL] testDSTestFailingAssertions() ([GAS])
Logs:
  assertionOne
  Error: a == b not satisfied [uint]
    Expected: 2
      Actual: 1
  assertionTwo
  Error: a == b not satisfied [uint]
    Expected: 4
      Actual: 3
  done

Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#,
    );
});

forgetest!(failing_setup, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "FailingSetupTest.t.sol",
        r#"
import "./test.sol";

contract FailingSetupTest is DSTest {
    event Test(uint256 n);

    function setUp() public {
        emit Test(42);
        require(false, "setup failed predictably");
    }

    function testShouldBeMarkedAsFailedBecauseOfSetup() public {
        emit log("setup did not fail");
    }
}
        "#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "FailingSetupTest"]).assert_failure().stdout_eq(str![[
        r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL: setup failed predictably] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#
    ]]);
});

forgetest!(multiple_after_invariants, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "MultipleAfterInvariantsTest.t.sol",
        r#"
import "./test.sol";

contract MultipleAfterInvariant is DSTest {
    function afterInvariant() public {}

    function afterinvariant() public {}

    function testFailShouldBeMarkedAsFailedBecauseOfAfterInvariant()
        public
        pure
    {
        assert(true);
    }
}
    "#,
    )
    .unwrap();

    cmd.args(["test", "--mc", "MultipleAfterInvariant"]).assert_failure().stdout_eq(str![[
        r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL: multiple afterInvariant functions] afterInvariant() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#
    ]]);
});

forgetest!(multiple_setups, |prj, cmd| {
    prj.insert_ds_test();

    prj.add_source(
        "MultipleSetupsTest.t.sol",
        r#"
    
import "./test.sol";

contract MultipleSetup is DSTest {
    function setUp() public {}

    function setup() public {}

    function testFailShouldBeMarkedAsFailedBecauseOfSetup() public {
        assert(true);
    }
}

    "#,
    )
    .unwrap();

    cmd.forge_fuse().args(["test", "--mc", "MultipleSetup"]).assert_failure().stdout_eq(str![[
        r#"[COMPILING_FILES] with [SOLC_VERSION]
...
[FAIL: multiple setUp functions] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
..."#
    ]]);
});

forgetest!(emit_diff_anonymous, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.add_source(
        "EmitDiffAnonymousTest.t.sol",
        r#"
    import "./test.sol";
    import "./Vm.sol";

    contract Target {
        event AnonymousEventNonIndexed(uint256 a) anonymous;

        function emitAnonymousEventNonIndexed(uint256 a) external {
            emit AnonymousEventNonIndexed(a);
        }
    }

    contract EmitDiffAnonymousTest is DSTest {
        Vm constant vm = Vm(HEVM_ADDRESS);
        Target target;

        event DifferentAnonymousEventNonIndexed(string a) anonymous;

        function setUp() public {
            target = new Target();
        }

        function testShouldFailEmitDifferentEventNonIndexed() public {
            vm.expectEmitAnonymous(false, false, false, false, true);
            emit DifferentAnonymousEventNonIndexed("1");
            target.emitAnonymousEventNonIndexed(1);
        }
    }
    "#,
    )
    .unwrap();

    cmd.forge_fuse().args(["test", "--mc", "EmitDiffAnonymousTest"]).assert_failure().stdout_eq(
        str![[r#"[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
...
[FAIL: log != expected log] testShouldFailEmitDifferentEventNonIndexed() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]],
    );
});
