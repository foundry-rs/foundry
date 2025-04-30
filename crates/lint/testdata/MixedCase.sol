// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract MixedCaseTest {
    // Passes
    uint256 variableMixedCase;
    uint256 _variableMixedCase;
    uint256 variablemixedcase;

    // Fails
    uint256 Variablemixedcase;
    uint256 VARIABLE_MIXED_CASE;
    uint256 VariableMixedCase;

    function foo() public {
        // Passes
        uint256 testVal;
        uint256 testVAL;
        uint256 testVal123;

        // Fails
        uint256 TestVal;
        uint256 TESTVAL;
    }

    // Passes
    function functionMixedCase() public {}
    function _functionMixedCase() internal {}
    function functionmixedcase() public {} 

    // Fails
    function Functionmixedcase() public {}
    function FUNCTION_MIXED_CASE() public {} 
    function FunctionMixedCase() public {} 
}
