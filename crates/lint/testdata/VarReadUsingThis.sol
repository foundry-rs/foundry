//@compile-flags: --only-lint var-read-using-this

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

library Lib {
    function foo() internal pure returns (uint256) {
        return 1;
    }
}

interface IThing {
    function value() external view returns (uint256);
}

contract Other {
    uint256 public value;
}

contract Base {
    uint256 public baseVar;

    function basePublicView() public view returns (uint256) {
        return baseVar;
    }
}

contract VarReadUsingThis is Base {
    uint256 public counter;
    mapping(uint256 => address) public owners;
    uint256[] public items;
    mapping(uint256 => mapping(address => uint256)) public balances;
    uint256 internal internalVar;
    uint256 private privateVar;

    // State-variable initializer is walked too (runs in the synthesized constructor).
    uint256 public initFromThis = uint256(uint160(address(this))) + this.counter(); //~NOTE: reading a state variable via `this`

    event Counted(uint256);
    error BadCount(uint256);

    struct Info {
        uint256 a;
        uint256 b;
    }
    Info public info;

    Other public other;

    modifier withArg(uint256) {
        _;
    }

    constructor() {
        // Reading `this` in the constructor reverts at runtime, but the lint still applies.
        counter = this.counter(); //~NOTE: reading a state variable via `this`
    }

    receive() external payable {
        items.push(this.counter()); //~NOTE: reading a state variable via `this`
    }

    // SHOULD FAIL:

    function simpleGetter() external view returns (uint256) {
        return this.counter(); //~NOTE: reading a state variable via `this`
    }

    function mappingGetter(uint256 k) external view returns (address) {
        return this.owners(k); //~NOTE: reading a state variable via `this`
    }

    function arrayGetter(uint256 i) external view returns (uint256) {
        return this.items(i); //~NOTE: reading a state variable via `this`
    }

    function nestedMappingGetter(uint256 a, address b) external view returns (uint256) {
        return this.balances(a, b); //~NOTE: reading a state variable via `this`
    }

    // Edge case: struct getter has multiple returns; emitted without an auto-fix.
    function structGetter() external view returns (uint256, uint256) {
        return this.info(); //~NOTE: reading a state variable via `this`
    }

    function publicViewLocal() public view returns (uint256) {
        return counter;
    }

    function callPublicView() external view returns (uint256) {
        return this.publicViewLocal(); //~NOTE: reading a state variable via `this`
    }

    // Improvement over Slither: external view/pure called via `this` is also flagged.
    function externalViewLocal() external view returns (uint256) {
        return counter;
    }

    function callExternalView() external view returns (uint256) {
        return this.externalViewLocal(); //~NOTE: reading a state variable via `this`
    }

    function inheritedStateVar() external view returns (uint256) {
        return this.baseVar(); //~NOTE: reading a state variable via `this`
    }

    function inheritedView() external view returns (uint256) {
        return this.basePublicView(); //~NOTE: reading a state variable via `this`
    }

    function parenAroundThis() external view returns (uint256) {
        return (this).counter(); //~NOTE: reading a state variable via `this`
    }

    function parenAroundCallee() external view returns (uint256) {
        return (this.counter)(); //~NOTE: reading a state variable via `this`
    }

    // Edge case: call options like `{gas: ...}` are flagged but no auto-fix is offered.
    function withCallOptions() external view returns (uint256) {
        return this.publicViewLocal{gas: 10000}(); //~NOTE: reading a state variable via `this`
    }

    modifier checkCounter() {
        require(this.counter() > 0, "zero"); //~NOTE: reading a state variable via `this`
        _;
    }

    function gated() external checkCounter {}

    // Modifier-invocation arguments are walked (this would be missed by a body-only walk).
    function gatedWithArg() external view withArg(this.counter()) returns (uint256) { //~NOTE: reading a state variable via `this`
        return 0;
    }

    function takesUint(uint256 x) public pure returns (uint256) {
        return x;
    }

    // Both inner and outer `this.X(...)` calls are flagged.
    function nestedCalls() external view returns (uint256) {
        return this.publicViewLocal() + this.counter(); //~NOTE: reading a state variable via `this`
        //~^NOTE: reading a state variable via `this`
    }

    function nestedAsArg() external view returns (uint256) {
        return this.takesUint(this.counter()); //~NOTE: reading a state variable via `this`
        //~^NOTE: reading a state variable via `this`
    }

    function inEmit() external {
        emit Counted(this.counter()); //~NOTE: reading a state variable via `this`
    }

    function inRevert() external view {
        revert BadCount(this.counter()); //~NOTE: reading a state variable via `this`
    }

    function inIfCondition() external view returns (uint256) {
        if (this.counter() > 0) { //~NOTE: reading a state variable via `this`
            return 1;
        }
        return 0;
    }

    function inTernary(bool b) external view returns (uint256) {
        return b ? this.counter() : this.publicViewLocal(); //~NOTE: reading a state variable via `this`
        //~^NOTE: reading a state variable via `this`
    }

    function inLoop(uint256 n) external view returns (uint256) {
        uint256 sum;
        for (uint256 i = 0; i < n; ++i) {
            sum += this.counter(); //~NOTE: reading a state variable via `this`
        }
        return sum;
    }

    function inUnchecked() external view returns (uint256) {
        unchecked {
            return this.counter() + 1; //~NOTE: reading a state variable via `this`
        }
    }

    // Inner `this.X(...)` inside a `try` argument is still flagged.
    function tryWithNestedRead() external returns (uint256) {
        try this.externalViewLocal() returns (uint256 v) {
            return v + this.counter(); //~NOTE: reading a state variable via `this`
        } catch {
            return 0;
        }
    }

    // SHOULD PASS:

    function directAccess() external view returns (uint256) {
        return counter;
    }

    // Edge case: function reference (no call) must not be flagged.
    function functionReference() external view returns (function() external view returns (uint256)) {
        return this.publicViewLocal;
    }

    function callOnOther() external view returns (uint256) {
        return other.value();
    }

    function publicMutating() public {
        counter += 1;
    }

    function callMutating() external {
        this.publicMutating();
    }

    function internalView() internal view returns (uint256) {
        return counter;
    }

    function callInternalView() external view returns (uint256) {
        return internalView();
    }

    // Edge case: `super` is a delegatecall mechanism, not `this`.
    function viaSuper() external view returns (uint256) {
        return super.basePublicView();
    }

    function callLib() external pure returns (uint256) {
        return Lib.foo();
    }

    // Edge case: `try this.X()` requires an external call by Solidity rules,
    // so the outer call is intentional and must not be flagged.
    function tryExternalView() external returns (uint256) {
        try this.externalViewLocal() returns (uint256 v) {
            return v;
        } catch {
            return 0;
        }
    }

    // Edge case: explicit interface cast through `address(this)` is not followed.
    function viaAddressThisCast() external view returns (uint256) {
        return Other(address(this)).value();
    }

    // Edge case: `this.balance` is a builtin address member, not a function call.
    function thisBalance() external view returns (uint256) {
        return address(this).balance;
    }

    // Edge case: `this.foo.selector` is a function-pointer member access, not a call.
    function selectorAccess() external view returns (bytes4) {
        return this.publicViewLocal.selector;
    }

    // Edge case: inline `disable-next-line` must suppress the diagnostic.
    function suppressed() external view returns (uint256) {
        // forge-lint: disable-next-line(var-read-using-this)
        return this.counter();
    }

    // Edge case: same-arity overloads with mixed mutability — solar's HIR doesn't
    // carry the resolved overload, so we conservatively skip flagging to avoid a
    // false positive on the mutating overload below.
    function ambiguous(uint256 x) public view returns (uint256) {
        return x;
    }
    function ambiguous(address) public {
        counter += 1;
    }
    function callAmbiguous() external view returns (uint256) {
        return this.ambiguous(0);
    }
}

// Abstract contracts have `this`, so the lint still applies.
abstract contract AbstractCase {
    uint256 public abstractVar;

    function readAbstract() external view returns (uint256) {
        return this.abstractVar(); //~NOTE: reading a state variable via `this`
    }
}
