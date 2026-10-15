//@compile-flags: --only-lint low-level-calls

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface Receiver {
    function ping(bytes calldata data) external returns (bool);
}

contract LowLevelCalls {
    event CallResult(bool, bytes);

    bytes existingData;

    function checkedCall(address target, bytes memory data) public {
        (bool success, bytes memory result) = target.call(data); //~NOTE: low-level call bypasses type checking
        require(success, "Call failed");
        emit CallResult(success, result);
    }

    function checkedCallWithValue(address payable target, uint256 value) public {
        (bool success, ) = target.call{value: value}(""); //~NOTE: low-level call bypasses type checking
        require(success, "Call failed");
    }

    function checkedDelegateCall(address target, bytes memory data) public returns (bool) {
        (bool success, ) = target.delegatecall(data); //~NOTE: low-level call bypasses type checking
        return success;
    }

    function checkedStaticCall(address target, bytes memory data) public view returns (bytes memory) {
        (bool success, bytes memory result) = target.staticcall(data); //~NOTE: low-level call bypasses type checking
        require(success, "Static call failed");
        return result;
    }

    function uncheckedCall(address target, bytes memory data) public {
        target.call(data); //~NOTE: low-level call bypasses type checking
    }

    function uncheckedCallWithValue(address payable target, uint256 value) public {
        target.call{value: value}(""); //~NOTE: low-level call bypasses type checking
    }

    function uncheckedDelegateCall(address target, bytes memory data) public {
        target.delegatecall(data); //~NOTE: low-level call bypasses type checking
    }

    function uncheckedStaticCall(address target, bytes memory data) public view {
        target.staticcall(data); //~NOTE: low-level call bypasses type checking
    }

    function ignoredSuccess(address target) public {
        (, existingData) = target.call(""); //~NOTE: low-level call bypasses type checking
    }

    function nonLowLevelMemberCalls(Receiver receiver, address payable target, bytes calldata data) public {
        receiver.ping(data);
        target.transfer(1 wei);
        bool sent = target.send(1 wei);
        require(sent, "Send failed");
    }
}
