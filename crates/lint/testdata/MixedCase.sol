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
    // `test`, `invariant`, and `statefulFuzz`
    function test_MixedCase_Disabled() public {}
    function testFuzz_MixedCase_Disabled() public {}
    function testRevert_MixedCase_Disabled() public {}
    function invariant_MixedCase_Disabled() public {}
    function invariantBalance_MixedCase_Disabled() public {}
    function statefulFuzz_MixedCase_Disabled() public {}
}
