//@compile-flags: --only-lint calls-loop

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.15;

interface IReceiver {
    function ping(uint256 value) external returns (bool);
    function purePing(uint256 value) external pure returns (bool);
}

interface IFactory {
    function getReceiver() external view returns (IReceiver);
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

    function call(Box storage box_, uint256 value) internal {
        box_.value = value;
    }

    function delegatecall(Box storage box_, uint256 value) internal {
        box_.value = value;
    }

    function staticcall(Box storage box_, uint256 value) internal view returns (uint256) {
        return box_.value + value;
    }
}

library ReceiverLib {
    function remember(IReceiver self, uint256 value) internal pure returns (uint256) {
        self;
        return value;
    }
}

struct Target {
    IReceiver receiver;
}

contract Receiver {
    event Ping(uint256 value);

    function ping(uint256 value) external returns (bool) {
        emit Ping(value);
        return true;
    }

    function purePing(uint256) external pure returns (bool) {
        return true;
    }
}

contract CallsLoop {
    using LocalLib for LocalLib.Box;
    using ReceiverLib for IReceiver;

    address payable[] public recipients;
    IReceiver[] public receivers;
    IReceiver public receiver;
    IFactory public factory;
    mapping(address => IReceiver) internal receiverByAddress;
    Target internal target;
    LocalLib.Box[] internal boxes;
    address[] internal scratchTargets;
    mapping(address => address[]) internal targetLists;

    modifier loopedPlaceholder() {
        for (uint256 i; i < 1; ++i) {
            _;
        }
    }

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

    function callsInterfaceCast(address targetAddress) external {
        for (uint256 i; i < receivers.length; ++i) {
            IReceiver(targetAddress).ping(i);
        }
    }

    function callsReturnedReceiver() external {
        for (uint256 i; i < receivers.length; ++i) {
            getReceiver().ping(i);
        }
    }

    function callsChainedReturnedReceiver() external {
        for (uint256 i; i < receivers.length; ++i) {
            factory.getReceiver().ping(i);
        }
    }

    function callsMappedReceiver(address targetAddress) external {
        for (uint256 i; i < receivers.length; ++i) {
            receiverByAddress[targetAddress].ping(i);
        }
    }

    function callsStructFieldReceiver() external {
        for (uint256 i; i < receivers.length; ++i) {
            target.receiver.ping(i);
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

    function callsThroughPureInternalHelper(IReceiver target_) external {
        for (uint256 i; i < receivers.length; ++i) {
            _pureNotify(target_, i);
        }
    }

    function callsThroughPublicHelper() external {
        for (uint256 i; i < receivers.length; ++i) {
            publicNotify(i);
        }
    }

    function selfExternalCall() external {
        for (uint256 i; i < receivers.length; ++i) {
            this.externalOnly(i);
        }
    }

    function callsThroughLoopedModifier() external loopedPlaceholder {
        receiver.ping(0);
    }

    function noLoopCall() external {
        receiver.ping(0);
    }

    function internalLibraryCallsAreIgnored() external {
        for (uint256 i; i < boxes.length; ++i) {
            boxes[i].ping(i);
            boxes[i].transfer(i);
            // forge-lint: disable-next-line(unchecked-call)
            boxes[i].call(i);
            // forge-lint: disable-next-line(unchecked-call)
            boxes[i].delegatecall(i);
            // forge-lint: disable-next-line(unchecked-call)
            boxes[i].staticcall(i);
        }
    }

    function internalLibraryExtensionOnInterfaceIsIgnored() external {
        for (uint256 i; i < receivers.length; ++i) {
            receiver.remember(i);
        }
    }

    function localInternalCallsAreIgnored() external {
        for (uint256 i; i < boxes.length; ++i) {
            _local(i);
        }
    }

    function arrayBuiltinsAreIgnored(address targetAddress) external {
        for (uint256 i; i < receivers.length; ++i) {
            scratchTargets.push(targetAddress);
        }
    }

    function mappingArrayBuiltinsAreIgnored(address targetKey, address targetAddress) external {
        for (uint256 i; i < receivers.length; ++i) {
            targetLists[targetKey].push(targetAddress);
        }
    }

    function externalOnly(uint256) external {}

    function getReceiver() internal view returns (IReceiver) {
        return receiver;
    }

    function _notify(uint256 value) internal {
        receiver.ping(value);
    }

    function _pureNotify(IReceiver target_, uint256 value) internal pure {
        target_.purePing(value);
    }

    function publicNotify(uint256 value) public {
        receiver.ping(value);
    }

    function _local(uint256 value) internal {
        boxes[0].value = value;
    }
}
