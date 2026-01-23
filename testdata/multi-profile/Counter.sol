// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

/// @notice A simple counter contract used to test multi-profile compilation.
/// The optimizer settings affect bytecode, so different profiles produce different bytecode.
contract Counter {
    uint256 public count;

    function increment() public {
        count += 1;
    }

    function decrement() public {
        count -= 1;
    }

    function setCount(uint256 _count) public {
        count = _count;
    }

    function getCount() public view returns (uint256) {
        return count;
    }

    /// @notice A function with some complexity to make optimizer differences visible
    function complexOperation(uint256 a, uint256 b) public pure returns (uint256) {
        uint256 result = 0;
        for (uint256 i = 0; i < 10; i++) {
            result += a * b + i;
        }
        return result;
    }
}
