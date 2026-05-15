//@compile-flags: --only-lint return-bomb

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IReturnBombTarget {
    function fetch() external returns (bytes memory);
    function value() external returns (uint256);
}

contract ReturnBomb {
    struct Result {
        bytes data;
    }

    bytes existingData;
    bytes[] results;
    Result storedResult;

    // SHOULD PASS: Calls without a gas cap.
    function valueOnlyOption(address payable target, bytes memory payload, uint256 value) public {
        (bool success, bytes memory result) = target.call{value: value}(payload);
        require(success, "Call failed");
        require(result.length >= 0);
    }

    // SHOULD FAIL: Gas-capped low-level calls that copy unbounded returndata.
    function ignoresReturnData(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, ) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
    }

    function standaloneLowLevelCall(address target, bytes memory payload, uint256 gasLimit) public {
        target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

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

    function directReturn(address target, bytes memory payload, uint256 gasLimit) public returns (bool, bytes memory) {
        return target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function memberBytes(address target, bytes memory payload, uint256 gasLimit) public {
        bool success;
        (success, storedResult.data) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
    }

    function indexedBytes(address target, bytes memory payload, uint256 gasLimit) public {
        bool success;
        results.push();
        (success, results[0]) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
    }

    function highLevelDynamicReturn(IReturnBombTarget target, uint256 gasLimit) public {
        bytes memory result = target.fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function standaloneHighLevelDynamicReturn(IReturnBombTarget target, uint256 gasLimit) public {
        target.fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function nestedHighLevelDynamicReturn(IReturnBombTarget target, uint256 gasLimit) public {
        require(target.fetch{gas: gasLimit}().length >= 0); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function directHighLevelDynamicReturn(IReturnBombTarget target, uint256 gasLimit) public returns (bytes memory) {
        return target.fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function highLevelStaticReturn(IReturnBombTarget target, uint256 gasLimit) public {
        uint256 result = target.value{gas: gasLimit}();
        require(result >= 0);
    }
}
