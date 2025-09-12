// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title DelegateCall - Testing delegate call traces
contract DelegateTarget {
    function fail() public pure {
        revert("Delegate target failed");
    }

    function compute(uint256 a, uint256 b) public pure returns (uint256) {
        require(a > 0, "a must be positive");
        require(b > 0, "b must be positive");
        return a + b;
    }
}

contract DelegateCaller {
    address public target;

    constructor(address _target) {
        target = _target;
    }

    function delegateFail() public {
        (bool success,) = target.delegatecall(abi.encodeWithSignature("fail()"));
        require(success, "Delegate call failed");
    }

    function delegateCompute(uint256 a, uint256 b) public returns (uint256) {
        (bool success, bytes memory data) =
            target.delegatecall(abi.encodeWithSignature("compute(uint256,uint256)", a, b));
        require(success, "Delegate compute failed");
        return abi.decode(data, (uint256));
    }
}
