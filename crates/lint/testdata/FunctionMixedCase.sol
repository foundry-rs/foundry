// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract FunctionMixedCaseTest {
    // Passes
    function functionMixedCase() public {}
    function _functionMixedCase() internal {}

    // Fails
    function Functionmixedcase() public {}
    function FUNCTION_MIXED_CASE() public {} 
    function functionmixedcase() public {} 
    function FunctionMixedCase() public {} 
}
