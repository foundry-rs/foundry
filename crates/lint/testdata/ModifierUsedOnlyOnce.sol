//@compile-flags: --only-lint modifier-used-only-once
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `modifier-used-only-once`: a modifier invoked by exactly one function can usually
// be inlined as checks at the top of that function. Invocations are taken from the resolved
// modifier lists, so base-constructor calls are not confused with modifier calls. Out of
// scope: `virtual` modifiers and overrides (they exist for dynamic dispatch), modifiers never
// invoked (dead code is another concern) or invoked more than once.

contract Guarded {
    address internal owner;
    uint256 internal count;

    modifier onlyOnce() { //~NOTE: this modifier is used only once
        require(msg.sender == owner, "not owner");
        _;
    }

    modifier usedTwice() {
        require(count < 100, "full");
        _;
    }

    modifier neverUsed() {
        require(owner != address(0), "unset");
        _;
    }

    function first() external onlyOnce usedTwice {
        count++;
    }

    function second() external usedTwice {
        count++;
    }
}

// A modifier invoked from a constructor counts like any other invocation.
contract Eager {
    bool internal ready;

    modifier once() { //~NOTE: this modifier is used only once
        require(!ready, "done");
        _;
    }

    constructor() once() {
        ready = true;
    }
}

// A `virtual` modifier and its override exist for dynamic dispatch: out of scope even when
// each is invoked once.
contract ModBase {
    modifier guard() virtual {
        _;
    }

    function useBase() external guard {}
}

contract ModChild is ModBase {
    modifier guard() override {
        require(msg.sender != address(0), "zero");
        _;
    }

    function useChild() external guard {}
}

// A base-constructor call sits in the same resolved list as modifier invocations: it must
// not be counted as one.
contract WithArgs {
    uint256 internal seed;

    constructor(uint256 s) {
        seed = s;
    }
}

contract ChildWithArgs is WithArgs {
    modifier seeded() { //~NOTE: this modifier is used only once
        require(seed != 0, "no seed");
        _;
    }

    constructor() WithArgs(42) {}

    function go() external seeded {}
}
