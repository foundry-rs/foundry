// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title StaticCall - Testing static call traces
contract StaticTarget {
    function viewFail() public pure {
        revert("Static call failed");
    }

    function compute(uint256 value) public pure returns (uint256) {
        require(value > 0, "Value must be positive");
        return value * 2;
    }
}

contract StaticCaller {
    address public target;

    constructor(address _target) {
        target = _target;
    }

    function staticCallFail() public view {
        (bool success,) = target.staticcall(abi.encodeWithSignature("viewFail()"));
        require(success, "Static call reverted");
    }

    function staticCompute(uint256 value) public view returns (uint256) {
        (bool success, bytes memory data) = target.staticcall(abi.encodeWithSignature("compute(uint256)", value));
        require(success, "Static compute failed");
        return abi.decode(data, (uint256));
    }
}
