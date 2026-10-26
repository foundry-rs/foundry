//@compile-flags: --only-lint unused-return
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract UnusedReturnFunctionPointer {
    function values() external pure returns (uint256, uint256) {
        return (1, 2);
    }

    function internalValue() internal pure returns (uint256) {
        return 1;
    }

    function noValue() external pure {}

    function externalCalls() external {
        function() external returns (uint256, uint256) callback = this.values;
        callback(); //~WARN: return value of an external call is not used
        (, uint256 value) = callback(); //~WARN: return value of an external call is not used
        (, value) = callback(); //~WARN: return value of an external call is not used
    }

    function externalParameter(function() external returns (uint256, uint256) callback) external {
        callback(); //~WARN: return value of an external call is not used
    }

    function noReturnValue() external {
        function() external callback = this.noValue;
        callback();
    }

    function internalCall() external pure {
        function() internal pure returns (uint256) callback = internalValue;
        callback();
    }
}
