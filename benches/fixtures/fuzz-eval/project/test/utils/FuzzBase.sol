// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Minimal cheatcode interface so the fixtures do not depend on forge-std.
interface Vm {
    function prank(address sender) external;
    function deal(address account, uint256 newBalance) external;
    function warp(uint256 timestamp) external;
}

Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

/// Clamps `x` into `[lo, hi]`, passing in-range values through unchanged.
function bound(uint256 x, uint256 lo, uint256 hi) pure returns (uint256) {
    if (x >= lo && x <= hi) return x;
    return lo + x % (hi - lo + 1);
}

/// Minimal invariant-test base exposing the target selection hooks forge reads.
abstract contract FuzzBase {
    address[] private targets;

    function targetContract(address target) internal {
        targets.push(target);
    }

    function targetContracts() public view returns (address[] memory) {
        return targets;
    }
}
