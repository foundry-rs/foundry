//! Tests for backtrace functionality

forgetest!(test_backtraces, |prj, cmd| {
    prj.insert_ds_test();
    prj.insert_vm();
    prj.add_source("SimpleRevert.sol", include_str!("../fixtures/backtraces/SimpleRevert.sol"));
    prj.add_source("StaticCall.sol", include_str!("../fixtures/backtraces/StaticCall.sol"));
    prj.add_source("DelegateCall.sol", include_str!("../fixtures/backtraces/DelegateCall.sol"));
    prj.add_source("NestedCalls.sol", include_str!("../fixtures/backtraces/NestedCalls.sol"));

    prj.add_test("Backtrace.t.sol", include_str!("../fixtures/backtraces/Backtrace.t.sol"));

    let output = cmd.args(["test", "-vvv"]).assert_failure();

    output.stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful with warnings:
...
Ran 11 tests for test/Backtrace.t.sol:BacktraceTest
[FAIL: panic: assertion failed (0x01)] testAssertFail() ([GAS])
...
Backtrace:
  at SimpleRevert.doAssert
  at BacktraceTest.testAssertFail (test/Backtrace.t.sol:40:48)

[FAIL: CustomError(42, 0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496)] testCustomError() ([GAS])
...
Backtrace:
  at SimpleRevert.doCustomError (src/SimpleRevert.sol:21:59)
  at BacktraceTest.testCustomError (test/Backtrace.t.sol:45:49)

[FAIL: Delegate compute failed] testDelegateCallRequire() ([GAS])
...
Backtrace:
  at DelegateTarget.compute (src/DelegateCall.sol:11:84)
  at DelegateCaller.delegateCompute (src/DelegateCall.sol:33:20)
  at BacktraceTest.testDelegateCallRequire (test/Backtrace.t.sol:82:57)

[FAIL: Delegate call failed] testDelegateCallRevert() ([GAS])
...
Backtrace:
  at DelegateTarget.fail (src/DelegateCall.sol:7:43)
  at DelegateCaller.delegateFail (src/DelegateCall.sol:26:91)
  at BacktraceTest.testDelegateCallRevert (test/Backtrace.t.sol:77:56)

[FAIL: Failed at internal level 3] testInternalCallChain() ([GAS])
...
Backtrace:
  at BacktraceTest.testInternalCallChain (test/Backtrace.t.sol:72:54)

[FAIL: Failed at chain level 3] testInternalCallsSameSource() ([GAS])
...
Backtrace:
  at NestedCalls.callChain1 (src/NestedCalls.sol:25:51)
  at BacktraceTest.testInternalCallsSameSource (test/Backtrace.t.sol:55:61)

[FAIL: Maximum depth reached] testNestedCalls() ([GAS])
...
Backtrace:
  at NestedCalls.nestedCall (src/NestedCalls.sol:11:46)
  at NestedCalls.nestedCall (src/NestedCalls.sol:13:19)
  at NestedCalls.nestedCall (src/NestedCalls.sol:13:19)
  at NestedCalls.nestedCall (src/NestedCalls.sol:13:19)
  at NestedCalls.nestedCall (src/NestedCalls.sol:13:19)
  at BacktraceTest.testNestedCalls (test/Backtrace.t.sol:50:49)

[FAIL: Value must be greater than zero] testRequireFail() ([GAS])
...
Backtrace:
  at SimpleRevert.doRequire (src/SimpleRevert.sol:11:61)
  at BacktraceTest.testRequireFail (test/Backtrace.t.sol:35:49)

[FAIL: Simple revert message] testSimpleRevert() ([GAS])
...
Backtrace:
  at SimpleRevert.doRevert (src/SimpleRevert.sol:7:67)
  at BacktraceTest.testSimpleRevert (test/Backtrace.t.sol:30:50)

[FAIL: Static compute failed] testStaticCallRequire() ([GAS])
...
Backtrace:
  at StaticTarget.compute (src/StaticCall.sol:11:77)
  at StaticCaller.staticCompute (src/StaticCall.sol:32:20)
  at BacktraceTest.testStaticCallRequire (test/Backtrace.t.sol:92:60)

[FAIL: Static call reverted] testStaticCallRevert() ([GAS])
...
Backtrace:
  at StaticTarget.viewFail (src/StaticCall.sol:7:47)
  at StaticCaller.staticCallFail (src/StaticCall.sol:25:93)
  at BacktraceTest.testStaticCallRevert (test/Backtrace.t.sol:87:59)

Suite result: FAILED. 0 passed; 11 failed; 0 skipped; [ELAPSED]
...
"#]]);
});
