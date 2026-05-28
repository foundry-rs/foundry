//@compile-flags: --only-lint write-after-write

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract WriteAfterWrite {
    uint256 public x;
    uint256 public y;
    mapping(address => uint256) public balances;

    // bad: first write to x is dead
    function bad1() public {
        x = 1;
        x = 2;
    }

    // bad: x written twice without read in between
    function bad2(uint256 v) public {
        x = 0;
        x = v;
    }

    // bad: delete followed by write is also a redundant delete
    function bad3(uint256 v) public {
        delete x;
        x = v;
    }

    // bad: parenthesized assignment statement is still caught
    function bad4(uint256 v) public {
        (x = 1);
        x = v;
    }

    // bad: nested assignment used as initializer; LHS is still a write
    function bad5_nested(uint256 v) public {
        x = 1;
        uint256 z = (x = v);
        z;
    }

    // bad: intra-branch write-after-write inside an if body
    function bad5(bool flag, uint256 v) public {
        if (flag) {
            x = 0;
            x = v;
        }
    }

    // good: x is read between writes
    function good1() public returns (uint256) {
        x = 1;
        return x;
    }

    // good: compound assignment reads x before writing
    function good2() public {
        x = 1;
        x += 1;
    }

    // good: x written, then y written (different vars)
    function good3(uint256 v) public {
        x = v;
        y = v;
    }

    // good: mapping writes may target different slots
    function good4(address a, address b, uint256 v) public {
        balances[a] = v;
        balances[b] = v;
    }

    // good: write in one branch, then write after the if (outer pending is cleared)
    function good5(bool flag, uint256 v) public {
        if (flag) {
            x = 1;
        }
        x = v;
    }

    // good: x used as RHS before second write
    function good6(uint256 v) public {
        x = 1;
        y = x + v;
        x = v;
    }

    // good: any function call conservatively clears pending
    function helper() internal view returns (uint256) { return x; }
    function good7(uint256 v) public {
        x = 1;
        helper();
        x = v;
    }

    // good: return before second write; first write is not dead
    function good8(uint256 v) public returns (uint256) {
        x = v;
        return x;
    }

    // good: revert before second write; first write is not dead
    function good9(uint256 v) public {
        x = v;
        revert();
    }

    // good: inner block shares pending with outer so reads inside count
    function good10(uint256 v) public {
        x = 1;
        { y = x + v; }
        x = v;
    }

    // bad: write in call argument is still overwritten before being read
    function bad6(uint256 v) public {
        x = 1;
        helper2(x = v);
    }
    function helper2(uint256) internal pure {}

    // good: ternary arms are exclusive; only one branch writes x
    function good11(bool flag) public {
        uint256 z = (flag ? (x = 1) : (x = 2));
        z;
    }

    // good: && short-circuits; RHS write may not execute
    function good12(uint256 v) public returns (bool) {
        x = 1;
        bool b = (v > 0) && (x = v) > 0;
        return b;
    }

    // bad: tuple destructuring writes x; second write to x is dead
    function bad7(uint256 v) public {
        (x, y) = (1, 2);
        x = v;
    }

    // bad: ++x writes x; x = v overwrites without reading
    function bad8(uint256 v) public {
        ++x;
        x = v;
    }

    // bad: x = 1; x++ reads then writes; x = v overwrites the x++ write
    function bad9(uint256 v) public {
        x = 1;
        x++;
        x = v;
    }

    // bad: write inside call option named arg overwrites pending write
    function bad10(address payable addr, uint256 v) public {
        x = 1;
        addr.call{value: (x = v)}("");
    }

    // bad: emit between two writes only reads args; pending should survive
    event MyEvent(uint256 v);
    function bad11(uint256 v) public {
        x = 1;
        emit MyEvent(v);
        x = v;
    }

    // good: no false positive for writes after a return (unreachable code)
    function good13(uint256 v) public returns (uint256) {
        x = v;
        return x;
        x = 1;
        x = 2;
    }

    // good: x++ reads x before writing, so prior x=1 write is live
    function good14() public returns (uint256) {
        x = 1;
        return x++;
    }

    // good: pre-inc reads x before writing; write between two is fine
    function good15(uint256 v) public {
        x = v;
        ++x;
    }
}

// good: modifier placeholder clears pending so writes on both sides are live
contract ModifierTest {
    uint256 public x;

    modifier guarded() {
        x = 1;
        _;
        x = 0;
    }

    function go() public guarded {}
}

// bad: write-after-write inside a modifier body
contract ModifierBad {
    uint256 public x;

    modifier badMod() {
        x = 1;
        x = 2;
        _;
    }
}
