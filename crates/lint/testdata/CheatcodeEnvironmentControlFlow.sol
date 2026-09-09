//@compile-flags: --only-lint environment-read-across-mutation
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface FlowClock {
    function roll(uint256 height) external;
    function warp(uint256 time) external;
}

library EnvironmentLibrary {
    function externalNumber() external view returns (uint256) { return block.number; }
    function externalTimestamp() external view returns (uint256) { return block.timestamp; }
    function publicNumber() public view returns (uint256) { return block.number; }

    function internalNumber() internal view returns (uint256) {
        return block.number; //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
    }

    function internalTimestamp() internal view returns (uint256) {
        return block.timestamp; //~WARN: `block.timestamp` may be reused across a Foundry environment mutation; capture it with `vm.getBlockTimestamp()` instead
    }
}

contract CheatcodeEnvironmentControlFlow {
    FlowClock constant vm = FlowClock(address(uint160(uint256(keccak256("hevm cheat code")))));

    function externalLibraryFrame() public returns (uint256, uint256, uint256) {
        uint256 number = EnvironmentLibrary.externalNumber();
        uint256 time = EnvironmentLibrary.externalTimestamp();
        uint256 publicNumber = EnvironmentLibrary.publicNumber();
        vm.roll(200);
        vm.warp(200);
        return (number, time, publicNumber);
    }

    function internalLibraryFrame() public returns (uint256, uint256) {
        uint256 number = EnvironmentLibrary.internalNumber();
        uint256 time = EnvironmentLibrary.internalTimestamp();
        vm.roll(200);
        vm.warp(200);
        return (number, time);
    }

    function oneIteration() public {
        for (uint256 i; i < 1; ++i) { vm.roll(block.number + 1); }
        for (uint256 i = 0; i != 1; i++) { vm.warp(block.timestamp + 1); }
    }

    function noIterations() public returns (uint256) {
        uint256 saved = block.number;
        for (uint256 i = 1; i < 1; ++i) { vm.roll(200); }
        return saved;
    }

    function countdown() public {
        for (uint256 i = 1; i > 0; --i) { vm.roll(block.number + 1); }
        for (uint256 i = 1; i >= 1; i--) { vm.warp(block.timestamp + 1); }
    }

    function compoundUpdate() public {
        for (uint256 i; i <= 0; i += 1) { vm.roll(block.number + 1); }
        for (uint256 i = 1; i != 0; i -= 1) { vm.warp(block.timestamp + 1); }
    }

    function whileUpdate() public {
        uint256 i;
        while (i < 1) { vm.roll(block.number + 1); ++i; }
        bool done;
        while (!done) { vm.warp(block.timestamp + 1); done = true; }
    }

    function postIncrementCondition() public {
        uint256 i;
        while (i++ < 1) { vm.roll(block.number + 1); }
    }

    function twoIterations() public {
        for (uint256 i; i < 2; ++i) {
            vm.roll(block.number + 1); //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
        }
        for (uint256 i; i < 2; i++) {
            vm.warp(block.timestamp + 1); //~WARN: `block.timestamp` may be reused across a Foundry environment mutation; capture it with `vm.getBlockTimestamp()` instead
        }
    }

    function afterOneIteration() public returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
        for (uint256 i; i < 1; ++i) { vm.roll(200); }
        return saved;
    }

    function unknownBound(uint256 count) public {
        for (uint256 i; i < count; ++i) {
            vm.roll(block.number + 1); //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
        }
    }

    function wrappedCounter() public {
        unchecked {
            for (uint8 i = 255; i > 0; ++i) { vm.roll(block.number + 1); }
            for (uint8 i; i == 0; --i) { vm.warp(block.timestamp + 1); }
        }
    }

    function checkedOverflow() public {
        for (uint8 i = 255; i > 0; ++i) { vm.roll(block.number + 1); }
    }

    function checkedIncrement(uint8 i) internal pure returns (uint8) { return i + 1; }

    function checkedHelperFromUnchecked() public {
        unchecked {
            for (uint8 i = 255; i > 0; i = checkedIncrement(i)) { vm.roll(block.number + 1); }
        }
    }

    function wrappingStillRepeats() public {
        unchecked {
            for (uint8 i = 255; i != 254; ++i) {
                vm.roll(block.number + 1); //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
            }
        }
    }

    function constantBoundAndAssignment() public {
        uint256 limit = 2 - 1;
        for (uint256 i; i < limit; i = i + 1) { vm.roll(block.number + 1); }
    }

    function doWhileOnce() public {
        uint256 i;
        do { vm.roll(block.number + 1); ++i; } while (i < 1);
    }

    function separateNumberOutcomes(bool choose, FlowClock nonVm) internal view returns (uint256, FlowClock) {
        if (choose) return (block.number, nonVm);
        return (0, vm);
    }

    function separateTimestampOutcomes(bool choose, FlowClock nonVm) internal view returns (uint256 saved, FlowClock receiver) {
        if (choose) { saved = block.timestamp; receiver = nonVm; }
        else { saved = 0; receiver = vm; }
    }

    function correlatedNumber(bool choose, FlowClock nonVm) public returns (uint256) {
        (uint256 saved, FlowClock receiver) = separateNumberOutcomes(choose, nonVm);
        receiver.roll(200);
        return saved;
    }

    function correlatedTimestamp(bool choose, FlowClock nonVm) public returns (uint256) {
        (uint256 saved, FlowClock receiver) = separateTimestampOutcomes(choose, nonVm);
        receiver.warp(200);
        return saved;
    }

    function correlatedConditional(bool choose, FlowClock nonVm) public returns (uint256) {
        (uint256 saved, FlowClock receiver) = choose ? (block.number, nonVm) : (uint256(0), vm);
        receiver.roll(200);
        return saved;
    }

    function hazardBeforeAmbiguousReturn(bool choose) internal returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
        if (choose) { vm.roll(200); return saved; }
        return 0;
    }

    function useAmbiguousReturn(bool choose) public returns (uint256) {
        return hazardBeforeAmbiguousReturn(choose);
    }

    function knownNumberOutcome() public returns (uint256, FlowClock) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across a Foundry environment mutation; capture it with `vm.getBlockNumber()` instead
        return (saved, vm);
    }

    function unambiguousTuple() public returns (uint256) {
        (uint256 saved, FlowClock receiver) = knownNumberOutcome();
        receiver.roll(200);
        return saved;
    }

    function identicalOutcomes(bool choose) internal view returns (uint256, FlowClock) {
        uint256 saved = block.timestamp; //~WARN: `block.timestamp` may be reused across a Foundry environment mutation; capture it with `vm.getBlockTimestamp()` instead
        if (choose) return (saved, vm);
        return (saved, vm);
    }

    function identicalTuple(bool choose) public returns (uint256) {
        (uint256 saved, FlowClock receiver) = identicalOutcomes(choose);
        receiver.warp(200);
        return saved;
    }
}
