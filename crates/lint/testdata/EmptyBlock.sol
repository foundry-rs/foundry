//@compile-flags: --only-lint empty-block
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `empty-block`: an empty body on a regular function is dead or unfinished code.
// Bodies whose emptiness is the behavior are exempt: constructors, receive/fallback (the empty
// body accepts calls or ether), `virtual` functions (an empty default meant to be overridden)
// and `payable` functions (an empty ether sink). Functions without a body (interfaces, abstract
// declarations) never fire. Nested empty blocks (`if (x) {}`) are out of scope.

// An empty free function is dead code.
function freeEmpty() pure {} //~WARN: empty function body

library EmptyBlockLib {
    function libEmpty(uint256 x) internal pure {} //~WARN: empty function body
}

abstract contract EmptyBlockBase {
    modifier noop() {
        _;
    }

    // Virtual with an empty default body: an intentional extension hook, exempt.
    function hook() internal virtual {}

    // Virtual without a body: nothing to flag.
    function toImplement() external virtual;

    // Virtual middle hook stays exempt even when it also overrides.
    function middleHook() internal virtual {}

    function _authorizeUpgrade(address newImplementation) internal virtual {}
}

contract EmptyBlockWithArg {
    constructor(uint256 x) {} // constructor: exempt, even with a parameter
}

contract EmptyBlock is EmptyBlockBase, EmptyBlockWithArg {
    uint256 internal value;

    constructor() EmptyBlockWithArg(1) {} // constructor with a base call: exempt

    receive() external payable {} // receive: exempt

    fallback() external payable {} // fallback: exempt

    // Payable with an empty body is an intentional ether sink.
    function deposit() external payable {}

    // Payable with a return value silently returns the default: an unfinished stub.
    function payableReturns() external payable returns (uint256) {} //~WARN: empty function body

    function emptyPublic() public {} //~WARN: empty function body

    function emptyExternal() external {} //~WARN: empty function body

    function emptyInternal() internal {} //~WARN: empty function body

    function emptyPrivate() private {} //~WARN: empty function body

    function emptyView() external view {} //~WARN: empty function body

    function emptyPure() external pure {} //~WARN: empty function body

    // A comment does not make the body non-empty.
    function commentOnly() public { /* nothing to do */ } //~WARN: empty function body

    // Overriding a parent declaration with an empty, non-virtual body is dead code.
    function toImplement() external override {} //~WARN: empty function body

    // The modifier carries the behavior: `initializer`-style and `onlyOwner`-style guards
    // commonly wrap an intentionally empty body.
    function withModifier() public noop {}

    // Virtual override: still an extension hook, exempt.
    function middleHook() internal virtual override {}

    // The UUPS pattern: the modifier is the whole point of the function.
    function _authorizeUpgrade(address newImplementation) internal override noop {}

    function nonEmpty() public {
        value = 1;
    }

    // Nested empty blocks are out of scope: the body is not empty.
    function nestedEmptyIf(uint256 x) public {
        if (x > 0) {}
        value = x;
    }
}
