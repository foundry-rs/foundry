//@compile-flags: --only-lint internal-function-used-once
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `internal-function-used-once`: an internal function called exactly once in the
// whole unit can usually be inlined into its caller. The single reference must be a direct
// call, resolved through the type checker; a reference used as a value (a function pointer
// assigned, returned or passed as a callback) has no call site to inline into and is out of
// scope. Also out of scope: functions whose name starts with `_` (hook convention), `virtual`
// functions and overrides (they exist for dynamic dispatch), functions referenced zero times
// (dead code is another concern) or more than once, and functions bound as user-defined
// operators (their operator uses are not name references, and the binding requires a named
// function).

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

    // Referenced exactly once, but as a value assigned to a function pointer rather than a
    // direct call: there is no call site to inline into, so it stays out of scope.
    function pointed(uint256 v) internal pure returns (uint256) {
        return v + 6;
    }

    function run(uint256 v) internal {
        count = helper(v) + twice(v) + _hooky(v) + recurse(v) + MathLib.onceQualified(v);
        count += v.attachedOnce();
    }

    function runAgain(uint256 v) internal {
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

// Functions bound as user-defined operators are out of scope: their operator uses are not
// name references, so their count would lie, and the binding requires a named function, so
// inlining is not an option. Both stay clean, with the operator as the only use or next to
// one direct call.
type Unit is uint256;

function operatorAdd(Unit a, Unit b) pure returns (Unit) {
    return Unit.wrap(Unit.unwrap(a) + Unit.unwrap(b));
}

function operatorSub(Unit a, Unit b) pure returns (Unit) {
    return Unit.wrap(Unit.unwrap(a) - Unit.unwrap(b));
}

using {operatorAdd as +, operatorSub as -} for Unit global;

contract UsesOperators {
    function viaOperatorOnly(Unit a, Unit b) internal pure returns (Unit) {
        return a + b;
    }

    function viaOperatorAndCall(Unit a, Unit b) internal pure returns (Unit) {
        return operatorSub(a - b, b);
    }
}

// Recursive functions cannot be inlined into a caller: a self-recursive helper stays out
// whether or not it has an external caller, and mutually recursive helpers whose only
// references are the cycle itself have no caller to inline into.
contract Recursion {
    function selfRecursiveNoCaller(uint256 v) internal pure returns (uint256) {
        if (v == 0) {
            return 0;
        }
        return selfRecursiveNoCaller(v - 1);
    }

    function factorial(uint256 v) internal pure returns (uint256) {
        if (v <= 1) {
            return 1;
        }
        return v * factorial(v - 1);
    }

    function useFactorial(uint256 v) internal pure returns (uint256) {
        return factorial(v);
    }

    function mutualEven(uint256 v) internal pure returns (bool) {
        if (v == 0) {
            return true;
        }
        return mutualOdd(v - 1);
    }

    function mutualOdd(uint256 v) internal pure returns (bool) {
        if (v == 0) {
            return false;
        }
        return mutualEven(v - 1);
    }
}

// A helper hanging off someone else's cycle is still inlineable: the cycle suppression only
// applies when the cycle contains the candidate itself, not when the single-reference chain
// runs into a later cycle.
contract TailOffCycle {
    function tail(uint256 v) internal pure returns (uint256) { //~NOTE: this internal function is used only once
        return v + 1;
    }

    function cycleA(uint256 v) internal pure returns (uint256) {
        if (v == 0) {
            return tail(v);
        }
        return cycleB(v - 1);
    }

    function cycleB(uint256 v) internal pure returns (uint256) {
        return cycleA(v);
    }
}

// The remaining value positions: a helper returned as a function pointer and one passed as a
// callback argument. Neither is a call site to inline into, so both stay out of scope, while
// the function actually called once (`invoke`) is reported.
contract ValuePositions {
    function returned(uint256 v) internal pure returns (uint256) {
        return v + 1;
    }

    function passed(uint256 v) internal pure returns (uint256) {
        return v + 2;
    }

    function invoke( //~NOTE: this internal function is used only once
        function(uint256) internal pure returns (uint256) cb,
        uint256 v
    ) internal pure returns (uint256) {
        return cb(v);
    }

    function pickReturned()
        internal
        pure
        returns (function(uint256) internal pure returns (uint256))
    {
        return returned;
    }

    function usePassed(uint256 v) internal pure returns (uint256) {
        return invoke(passed, v);
    }
}
