//@compile-flags: --only-lint return-bomb

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract ReturnBomb {
    bytes existingData;

    // SHOULD PASS: Gas-capped calls that do not bind returndata.
    function ignoresReturnData(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, ) = target.call{gas: gasLimit}(payload);
        require(success, "Call failed");
    }

    function valueOnlyOption(address payable target, bytes memory payload, uint256 value) public {
        (bool success, bytes memory result) = target.call{value: value}(payload);
        require(success, "Call failed");
        require(result.length >= 0);
    }

    // SHOULD FAIL: Gas-capped calls that bind unbounded returndata.
    function lowLevelCall(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, bytes memory result) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
        require(result.length >= 0);
    }

    function lowLevelCallLiteralGas(address target, bytes memory payload) public {
        (bool success, bytes memory result) = target.call{gas: 10_000}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
        require(result.length >= 0);
    }

    function lowLevelCallWithValue(address payable target, bytes memory payload, uint256 gasLimit, uint256 value) public {
        (bool success, bytes memory result) = target.call{value: value, gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
        require(result.length >= 0);
    }

    function delegateCall(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, bytes memory result) = target.delegatecall{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Delegatecall failed");
        require(result.length >= 0);
    }

    function staticCall(address target, bytes memory payload, uint256 gasLimit) public view {
        (bool success, bytes memory result) = target.staticcall{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Staticcall failed");
        require(result.length >= 0);
    }

    function existingBytes(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, bytes memory result) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        (success, existingData) = target.call{gas: gasLimit}(result); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
    }
}
