// SPDX-License-Identifier: MIT
pragma solidity ^0.8.15;

interface IReceiver {
    function ping(uint256 value) external returns (bool);
}

library LocalLib {
    struct Box {
        uint256 value;
    }

    function ping(Box storage box_, uint256 value) internal {
        box_.value = value;
    }

    function transfer(Box storage box_, uint256 value) internal {
        box_.value = value;
    }
}

contract Receiver {
    event Ping(uint256 value);

    function ping(uint256 value) external returns (bool) {
        emit Ping(value);
        return true;
    }
}

contract CallsLoop {
    using LocalLib for LocalLib.Box;

    address payable[] public recipients;
    IReceiver[] public receivers;
    IReceiver public receiver;
    LocalLib.Box[] internal boxes;

    function lowLevelCalls(address[] calldata targets) external {
        for (uint256 i; i < targets.length; ++i) {
            (bool success,) = targets[i].call("");
            require(success);
        }
    }

    function ethTransfers() external {
        uint256 i;
        while (i < recipients.length) {
            recipients[i].transfer(1 wei);
            i++;
        }
    }

    function highLevelCalls() external {
        for (uint256 i; i < receivers.length; ++i) {
            receivers[i].ping(i);
        }
    }

    function tryCalls() external {
        for (uint256 i; i < receivers.length; ++i) {
            try receivers[i].ping(i) returns (bool) {} catch {}
        }
    }

    function callsThroughInternalHelper() external {
        for (uint256 i; i < receivers.length; ++i) {
            _notify(i);
        }
    }

    function selfExternalCall() external {
        for (uint256 i; i < receivers.length; ++i) {
            this.externalOnly(i);
        }
    }

    function noLoopCall() external {
        receiver.ping(0);
    }

    function internalLibraryCallsAreIgnored() external {
        for (uint256 i; i < boxes.length; ++i) {
            boxes[i].ping(i);
            boxes[i].transfer(i);
        }
    }

    function localInternalCallsAreIgnored() external {
        for (uint256 i; i < boxes.length; ++i) {
            _local(i);
        }
    }

    function externalOnly(uint256) external {}

    function _notify(uint256 value) internal {
        receiver.ping(value);
    }

    function _local(uint256 value) internal {
        boxes[0].value = value;
    }
}
