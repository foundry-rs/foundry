//@compile-flags: --only-lint calls-loop

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.15;

interface IDispatchTarget {
    function ping() external;
}

contract CallsLoopDispatchBase {
    function hook(IDispatchTarget target) internal virtual {}

    function inheritedLoop(IDispatchTarget target) external {
        for (uint256 i; i < 1; ++i) {
            hook(target);
        }
    }
}

contract CallsLoopDispatchLeaf is CallsLoopDispatchBase {
    function hook(IDispatchTarget target) internal override {
        target.ping(); //~WARN: external call inside a loop
    }
}

contract CallsLoopFunctionPointer {
    function noop(IDispatchTarget) internal {}

    function callsTarget(IDispatchTarget target) internal {
        target.ping(); //~WARN: external call inside a loop
    }

    function pointerLoop(IDispatchTarget target, bool callTarget) external {
        function(IDispatchTarget) internal callback = callTarget ? callsTarget : noop;
        for (uint256 i; i < 1; ++i) {
            callback(target);
        }
    }
}
