// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract VariableMixedCase {
    uint256 variableMixedCase;
    uint256 _variableMixedCase;
    uint256 Variablemixedcase;
    uint256 VARIABLE_MIXED_CASE;
    uint256 variablemixedcase;

    function foo() public {
        uint256 testVal = 1;
        uint256 testVAL = 2;
        uint256 TestVal = 3;
        uint256 TESTVAL = 4;
        uint256 tESTVAL = 5;
        uint256 test6Val = 6;
    }
}
