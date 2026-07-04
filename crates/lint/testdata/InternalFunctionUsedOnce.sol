//@compile-flags: --only-lint internal-function-used-once
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `internal-function-used-once`: an internal function referenced exactly once in the
// whole unit can usually be inlined into its caller. References are resolved through the type
// checker, calls and values alike. Out of scope: functions whose name starts with `_` (hook
// convention), `virtual` functions and overrides (they exist for dynamic dispatch), functions
// referenced zero times (dead code is another concern) or more than once.

library MathLib {
    function onceQualified(uint256 v) internal pure returns (uint256) { //~NOTE: this internal function is used only once
        return v + 1;
    }

    function attachedOnce(uint256 v) internal pure returns (uint256) { //~NOTE: this internal function is used only once
        return v + 2;
    }
}

contract Counter {
    using MathLib for uint256;

    uint256 internal count;

    function helper(uint256 v) internal pure returns (uint256) { //~NOTE: this internal function is used only once
        return v * 2;
    }

    function twice(uint256 v) internal pure returns (uint256) {
        return v + 3;
    }

    function _hooky(uint256 v) internal pure returns (uint256) {
        return v + 4;
    }

    function neverUsed(uint256 v) internal pure returns (uint256) {
        return v + 5;
    }

    function recurse(uint256 v) internal pure returns (uint256) {
        // the self-reference counts too: one external caller makes two references
        return v == 0 ? 0 : recurse(v - 1);
    }

    function pointed(uint256 v) internal pure returns (uint256) { //~NOTE: this internal function is used only once
        return v + 6;
    }

    function run(uint256 v) internal {
        count = helper(v) + twice(v) + _hooky(v) + recurse(v) + MathLib.onceQualified(v);
        count += v.attachedOnce();
    }

    function runAgain(uint256 v) internal {
        // a reference used as a value is a reference like any other
        function(uint256) internal pure returns (uint256) f = pointed;
        count = twice(f(v));
    }
}

// A virtual function and its override exist for dynamic dispatch: out of scope even when
// referenced once.
contract VirtualBase {
    function pick(uint256 v) internal view virtual returns (uint256) {
        return v + block.number;
    }

    function useBase(uint256 v) internal view returns (uint256) {
        // the single static reference to the virtual base: exempt all the same
        return pick(v);
    }
}

contract VirtualChild is VirtualBase {
    function pick(uint256 v) internal view override returns (uint256) {
        return v + block.timestamp;
    }

    function use(uint256 v) internal view returns (uint256) {
        return pick(v);
    }
}

// A free function is internal by nature and counted across the unit.
function freeOnce(uint256 v) pure returns (uint256) { //~NOTE: this internal function is used only once
    return v + 7;
}

contract UsesFree {
    function go(uint256 v) internal pure returns (uint256) {
        return freeOnce(v);
    }
}
