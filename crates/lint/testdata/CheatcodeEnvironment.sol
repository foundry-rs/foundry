//@compile-flags: --only-lint environment-read-across-mutation
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface Clock {
    function roll(uint256 height) external;
    function warp(uint256 time) external;
    function getBlockNumber() external view returns (uint256);
    function getBlockTimestamp() external view returns (uint256);
}

interface WrongSignature {
    function roll(bytes32 height) external;
    function warp(uint64 time) external;
}

contract ClockHelpers {
    Clock internal constant clock = Clock(address(uint160(uint256(keccak256("hevm cheat code")))));

    function advance(uint256 height) internal { clock.roll(height); }
    function skip(uint256 delta) internal { clock.warp(clock.getBlockTimestamp() + delta); }
    function rewind(uint256 delta) internal { clock.warp(clock.getBlockTimestamp() - delta); }
    function identity(uint256 value) internal pure returns (uint256) { return value; }
    function advanceReceiver(Clock receiver) internal { receiver.roll(200); }

    modifier warpFirst() { clock.warp(200); _; }
    modifier warpLast() { _; clock.warp(200); }
    modifier optionalWarp() virtual { clock.warp(200); _; }
    function virtualModifier() internal optionalWarp {}

    function inheritedRead() internal view returns (uint256) {
        return block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
    }
}

contract CheatcodeEnvironment is ClockHelpers {
    modifier optionalWarp() override { _; }

    function explicitModifier() internal ClockHelpers.optionalWarp {}
    function restoreNumber() public {
        clock.roll(100);
        uint256 saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        clock.roll(200);
        clock.roll(saved);
    }

    function restoreTimestamp() public {
        clock.warp(100);
        uint256 saved = block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        clock.warp(200);
        clock.warp(saved);
    }

    function aliasesAndArithmetic() public returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        uint256 derived = identity(saved + 10);
        Clock alias_ = clock;
        alias_.roll(200);
        return derived;
    }

    function readsOnBothSides() public returns (uint256, uint256) {
        uint256 before_ = block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        skip(20);
        return (before_, block.timestamp);
    }

    function inheritedHelper() public returns (uint256) {
        uint256 saved = inheritedRead();
        advance(200);
        return saved;
    }

    function receiverParameter() public returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        advanceReceiver(clock);
        return saved;
    }

    function namedArgument() public returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        advanceReceiver({receiver: clock});
        return saved;
    }

    function rewindHelper() public returns (uint256) {
        uint256 saved = block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        rewind(1);
        return saved;
    }

    function mutateInModifier() internal warpFirst {}

    function modifierCall() public returns (uint256) {
        uint256 saved = block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        mutateInModifier();
        return saved;
    }

    function modifierSuffix() public warpLast returns (uint256) {
        return block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
    }

    function tupleAlias() public returns (uint256) {
        (uint256 saved, uint256 other) = (block.number, 7); //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        (other, saved) = (saved, other);
        clock.roll(200);
        return other;
    }

    function reachableBranch(bool choose) public returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        if (choose) clock.roll(200);
        return saved;
    }

    function loopCarried(bool choose) public {
        uint256 saved = block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        while (choose) {
            clock.warp(saved + 1);
        }
    }

    // Safe materialization, unrelated calls, overwrites, and unreachable paths stay quiet.

    function getters() public returns (uint256, uint256) {
        uint256 height = clock.getBlockNumber();
        uint256 time = clock.getBlockTimestamp();
        clock.roll(200);
        clock.warp(200);
        return (height, time);
    }

    function firstReadsAfterMutation() public returns (uint256, uint256) {
        clock.roll(200);
        clock.warp(200);
        return (block.number, block.timestamp);
    }

    function separateEnvironments() public returns (uint256) {
        uint256 saved = block.number;
        clock.warp(200);
        return saved;
    }

    function separateEnvironmentsTime() public returns (uint256) {
        uint256 saved = block.timestamp;
        clock.roll(200);
        return saved;
    }

    function ordinaryReceiver(Clock vm) public returns (uint256, uint256) {
        uint256 height = block.number;
        uint256 time = block.timestamp;
        vm.roll(200);
        vm.warp(200);
        return (height, time);
    }

    function unrelatedSignature() public returns (uint256, uint256) {
        uint256 height = block.number;
        uint256 time = block.timestamp;
        WrongSignature vm = WrongSignature(address(clock));
        vm.roll(bytes32(uint256(200)));
        vm.warp(uint64(200));
        return (height, time);
    }

    function overwritten() public returns (uint256) {
        uint256 saved = block.number;
        clock.roll(200);
        saved = 123;
        return saved;
    }

    function tupleOverwrite() public returns (uint256) {
        (uint256 saved, uint256 safe) = (block.number, 7);
        clock.roll(200);
        (saved, safe) = (safe, 12);
        return saved;
    }

    function oppositeBranches(bool choose) public returns (uint256) {
        uint256 saved;
        if (choose) saved = block.number;
        else clock.roll(200);
        return saved;
    }

    function returningBranch(bool choose) public returns (uint256) {
        uint256 saved = block.number;
        if (choose) {
            clock.roll(200);
            return 0;
        }
        return saved;
    }

    function revertingBranch(bool choose) public returns (uint256) {
        uint256 saved = block.timestamp;
        if (choose) {
            clock.warp(200);
            revert();
        }
        return saved;
    }

    function unreachableMutation() public returns (uint256) {
        uint256 saved = block.number;
        if (false) clock.roll(200);
        return saved;
    }

    function receiverOverwritten(Clock other) public returns (uint256) {
        uint256 saved = block.number;
        Clock alias_ = clock;
        alias_ = other;
        alias_.roll(200);
        return saved;
    }

    function externalRead() external view returns (uint256) { return block.number; }

    function separateFrame() public returns (uint256) {
        uint256 saved = this.externalRead();
        clock.roll(200);
        return saved;
    }

    function beforeOnly() public {
        clock.roll(block.number + 1);
    }

    function unusedCapture() public {
        uint256 saved = block.timestamp;
        clock.warp(200);
    }

    function namedReturn() public returns (uint256 saved) {
        saved = block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        clock.warp(200);
        return;
    }

    function implicitReturn() public returns (uint256 saved) {
        saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        clock.roll(200);
    }

    function helperTuple() internal view returns (uint256, uint256) {
        return (block.number, 7); //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
    }

    function returnedTuple() public returns (uint256) {
        (uint256 saved, uint256 other) = helperTuple();
        clock.roll(200);
        return saved + other;
    }

    function noLoopIterations() public returns (uint256) {
        uint256 saved = block.number;
        while (false) clock.roll(200);
        return saved;
    }

    function breakingLoop() public returns (uint256) {
        uint256 saved = block.number;
        while (true) {
            break;
            clock.roll(200);
        }
        return saved;
    }

    function continuingLoop(bool choose) public returns (uint256) {
        uint256 saved = block.number;
        while (choose) {
            continue;
            clock.roll(200);
        }
        return saved;
    }

    function receiverTruncated() public returns (uint256) {
        uint256 saved = block.number;
        uint256 address_ = uint256(uint160(address(clock)));
        Clock(address(uint160(uint8(address_)))).roll(200);
        return saved;
    }

    function receiverIncremented() public returns (uint256) {
        uint256 saved = block.number;
        uint256 address_ = uint256(uint160(address(clock)));
        address_++;
        Clock(address(uint160(address_))).roll(200);
        return saved;
    }

    function deletedCapture() public returns (uint256) {
        uint256 saved = block.number;
        clock.roll(200);
        delete saved;
        return saved;
    }

    function failingRoll() public returns (uint256) {
        uint256 saved = block.number;
        try clock.roll(200) {
            return 0;
        } catch {
            return saved;
        }
    }

    function successfulWarp() public returns (uint256) {
        uint256 saved = block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        try clock.warp(200) {
            return saved;
        } catch {
            return 0;
        }
    }

    function compoundOverwrite() public returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        saved += 1;
        clock.roll(200);
        return saved;
    }

    function assemblyOverwrite() public returns (uint256) {
        uint256 saved = block.number;
        clock.roll(200);
        assembly { saved := 7 }
        return saved;
    }

    function qualifiedModifier() public returns (uint256) {
        uint256 saved = block.timestamp; //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        explicitModifier();
        return saved;
    }

    function overriddenModifier() public returns (uint256) {
        uint256 saved = block.timestamp;
        virtualModifier();
        return saved;
    }

    function suppressedTime() public returns (uint256) {
        // forge-lint: disable-next-line(environment-read-across-mutation)
        uint256 saved = block.timestamp;
        clock.warp(200);
        return saved;
    }

    function suppressed() public returns (uint256) {
        // forge-lint: disable-next-line(environment-read-across-mutation)
        uint256 saved = block.number;
        clock.roll(200);
        return saved;
    }
}
