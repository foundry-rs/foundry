//@compile-flags: --only-lint environment-read-across-mutation
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface PendingClock {
    function roll(uint256 height) external;
    function warp(uint256 time) external;
}

contract CheatcodeEnvironmentPending {
    PendingClock constant vm = PendingClock(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256[2] a;

    function advance() internal returns (uint256) { vm.roll(200); return 0; }
    function advanceTime() internal returns (uint256) { vm.warp(200); return 0; }
    function first(uint256 value, uint256) internal pure returns (uint256) { return value; }

    function assignment() public returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        a[advance()] = 1;
        return saved;
    }

    function deletion() public returns (uint256) {
        uint256 saved = block.number; //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        delete a[advance()];
        return saved;
    }

    function tuple() public returns (uint256) {
        (uint256 saved, uint256 ignored) = (block.number, advance()); //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
        return saved + ignored;
    }

    function binary() public returns (uint256) {
        return block.number + advance(); //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
    }

    function arguments() public returns (uint256) {
        return first(block.number, advance()); //~WARN: `block.number` may be reused across `vm.roll`; capture it with `vm.getBlockNumber()` instead
    }

    function timestamp() public returns (uint256) {
        (uint256 saved, uint256 ignored) = (block.timestamp, advanceTime()); //~WARN: `block.timestamp` may be reused across `vm.warp`; capture it with `vm.getBlockTimestamp()` instead
        return saved + ignored;
    }

    function overwritten() public returns (uint256) {
        uint256 saved = block.number;
        vm.roll(200);
        saved = 1;
        delete saved;
        return saved;
    }

    function fresh() public returns (uint256) {
        (uint256 ignored, uint256 saved) = (advance(), block.number);
        return saved + ignored;
    }

    function freshBranches(bool flag) public returns (uint256) {
        uint256 saved;
        if (flag) {
            vm.roll(200);
            saved = block.number;
        } else {
            saved = block.number;
        }
        return saved;
    }

    function otherEnvironment() public returns (uint256) {
        return first(block.number, advanceTime());
    }
}
