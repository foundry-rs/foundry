//@compile-flags: --only-lint missing-events-access-control

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract MissingEventsAccessControlForUpdate {
    address public owner = msg.sender;

    event OwnershipTransferred(address oldOwner, address newOwner);

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    // The update expression is part of the loop and must be checked for writes.
    function writeInUpdate(address newOwner, bool enabled) external onlyOwner {
        for (; enabled; owner = newOwner) { //~WARN: `owner` is changed without an event but is used for access control
            enabled = false;
        }
    }

    function helperWriteInUpdate(address newOwner, bool enabled) external onlyOwner {
        for (; enabled; _setOwner(newOwner)) {
            enabled = false;
        }
    }

    // An event in the update can cover a write in the same iteration.
    function eventInUpdate(address newOwner, bool enabled) external onlyOwner {
        for (; enabled; _logOwner(newOwner)) {
            owner = newOwner;
            enabled = false;
        }
    }

    // An event in the body can also cover a write in the update.
    function eventBeforeUpdate(address newOwner, bool enabled) external onlyOwner {
        for (; enabled; owner = newOwner) {
            emit OwnershipTransferred(owner, newOwner);
            enabled = false;
        }
    }

    // The update may never run, so its event cannot cover an outside write.
    function conditionalEventInUpdate(address newOwner, bool enabled) external onlyOwner {
        for (; enabled; _logOwner(newOwner)) {
            enabled = false;
        }
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function writeBeforeConditionalUpdate(address newOwner, bool enabled) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        for (; enabled; _logOwner(newOwner)) {
            enabled = false;
        }
    }

    // A nested conditional event cannot cover every path to the update.
    function conditionalBodyEvent(address newOwner, bool enabled, bool logEvent) external onlyOwner {
        for (; enabled; owner = newOwner) { //~WARN: `owner` is changed without an event but is used for access control
            if (logEvent) emit OwnershipTransferred(owner, newOwner);
            enabled = false;
        }
    }

    // Loops without a condition still visit their update expression.
    function writeInUnconditionalUpdate(address newOwner, bool stop) external onlyOwner {
        for (;; owner = newOwner) { //~WARN: `owner` is changed without an event but is used for access control
            if (stop) break;
            stop = true;
        }
    }

    function _setOwner(address newOwner) internal {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function _logOwner(address newOwner) internal {
        emit OwnershipTransferred(owner, newOwner);
    }
}
