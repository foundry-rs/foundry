// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract UncheckedCall {

    // SHOULD PASS: Properly checked low-level calls
    function checkedCallWithTuple(address target, bytes memory data) public {
        (bool success, bytes memory result) = target.call(data);
        require(success, "Call failed");
        emit CallResult(success, result);
    }

    function checkedCallWithIfStatement(address target, bytes memory data) public {
        (bool success, ) = target.call(data);
        if (!success) {
            revert("Call failed");
        }
    }

    function checkedDelegateCall(address target, bytes memory data) public returns (bool) {
        (bool success, ) = target.delegatecall(data);
        return success;
    }

    function checkedStaticCall(address target, bytes memory data) public view returns (bytes memory) {
        (bool success, bytes memory result) = target.staticcall(data);
        require(success, "Static call failed");
        return result;
    }

    function checkedCallInRequire(address target) public {
        (bool success, ) = target.call("");
        require(success, "Call must succeed");
    }

    function checkedCallWithAssert(address target) public {
        (bool success, ) = target.call("");
        assert(success);
    }

    // Edge case: pre-existing variable assignment
    bool success;
    function checkWithExistingVar(address target) public {
        (bool, ) = target.call("");
        (bool, existingData) = target.call("");
    }

    // Edge case: send and transfer are not low-level calls (they automatically revert on failure)
    function sendEther(address payable target) public {
        target.transfer(1 ether); // Should not trigger
        bool sent = target.send(1 ether); // Should not trigger
        require(sent, "Send failed");
    }


    // SHOULD FAIL: Unchecked low-level calls
    function uncheckedCall(address target, bytes memory data) public {
        target.call(data); //~WARN: Low-level calls should check the success return value
    }

    function uncheckedCallWithValue(address payable target, uint256 value) public {
        target.call{value: value}(""); //~WARN: Low-level calls should check the success return value
    }

    function uncheckedDelegateCall(address target, bytes memory data) public {
        target.delegatecall(data); //~WARN: Low-level calls should check the success return value
    }

    function uncheckedStaticCall(address target, bytes memory data) public {
        target.staticcall(data); //~WARN: Low-level calls should check the success return value
    }

    function multipleUncheckedCalls(address target1, address target2) public {
        target1.call(""); //~WARN: Low-level calls should check the success return value
        target2.delegatecall(""); //~WARN: Low-level calls should check the success return value
    }

    function ignoredReturnWithPartialTuple(address target) public {
        (, bytes memory data) = target.call(""); //~WARN: Low-level calls should check the success return value
        // Only capturing data, not checking success
    }

    bytes memory existingData;
    function ignoredReturnExistingVar(address target) public {
        (, existingData) = target.call(""); //~WARN: Low-level calls should check the success return value
    }

}
