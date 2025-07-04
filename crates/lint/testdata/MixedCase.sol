// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract MixedCaseTest {
    uint256 variableMixedCase;
    uint256 _variableMixedCase;
    uint256 variablemixedcase;

    uint256 Variablemixedcase; //~NOTE: mutable variables should use mixedCase
    uint256 VARIABLE_MIXED_CASE; //~NOTE: mutable variables should use mixedCase
    uint256 VariableMixedCase; //~NOTE: mutable variables should use mixedCase

    function foo() public {
        uint256 testVal;
        uint256 testVal123;

        uint256 testVAL; //~NOTE: mutable variables should use mixedCase
        uint256 TestVal; //~NOTE: mutable variables should use mixedCase
        uint256 TESTVAL; //~NOTE: mutable variables should use mixedCase
    }

    function functionMixedCase() public {}
    function _functionMixedCase() internal {}
    function functionmixedcase() public {}

    function Functionmixedcase() public {} //~NOTE: function names should use mixedCase
    function FUNCTION_MIXED_CASE() public {} //~NOTE: function names should use mixedCase
    function FunctionMixedCase() public {} //~NOTE: function names should use mixedCase
    function function_mixed_case() public {} //~NOTE: function names should use mixedCase

    // mixedCase checks are disabled for functions that starting with:
    // `test`, `invariant_`, and `statefulFuzz`
    function test_MixedCase_Disabled() public {}
    function test_mixedcase_disabled() public {}
    function testMixedCaseDisabled() public {}
    function testmixedcasedisabled() public {}

    function testFuzz_MixedCase_Disabled() public {}
    function testFuzz_mixedcase_disabled() public {}
    function testFuzzMixedCaseDisabled() public {}
    function testfuzzmixedcasedisabled() public {}

    function testRevert_MixedCase_Disabled() public {}
    function testRevert_mixedcase_disabled() public {}
    function testRevertMixedCaseDisabled() public {}
    function testrevertmixedcasedisabled() public {}

    function invariant_MixedCase_Disabled() public {}
    function invariant_mixedcase_disabled() public {}
    function invariant_MixedCaseDisabled() public {}
    function invariant_mixedcasedisabled() public {}

    function invariantBalance_MixedCase_Enabled() public {} //~NOTE: function names should use mixedCase
    function invariantbalance_mixedcase_enabled() public {} //~NOTE: function names should use mixedCase
    function invariantBalanceMixedCaseEnabled() public {}
    function invariantbalancemixedcaseenabled() public {}

    function statefulFuzz_MixedCase_Disabled() public {}
    function statefulFuzz_mixedcase_disabled() public {}
    function statefulFuzzMixedCaseDisabled() public {}
    function statefulFuzzmixedcasedisabled() public {}
}
