//@compile-flags: --only-lint uninitialized-local

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract UninitializedLocal {
    struct Point {
        uint256 x;
        uint256 y;
    }

    address public owner;

    // SHOULD WARN:

    // Classic slither example: `to` is never assigned, defaults to address(0).
    function withdraw() public {
        address payable to;
        to.transfer(address(this).balance); //~WARN: local variable is read before being initialized
    }

    // Returning an uninitialized uint silently returns 0.
    function getAmount() public pure returns (uint256) {
        uint256 amount;
        return amount; //~WARN: local variable is read before being initialized
    }

    // Reading an uninitialized var in an expression.
    function compute(uint256 b) public pure returns (uint256) {
        uint256 a;
        return a + b; //~WARN: local variable is read before being initialized
    }

    // Reading an uninitialized var in the initializer of another local.
    function alias_() public pure returns (address) {
        address to;
        address from = to; //~WARN: local variable is read before being initialized
        return from;
    }

    // Only one branch assigns, so still uninitialized on the path that skips the branch.
    function conditional(bool flag, address addr) public {
        address payable to;
        if (flag) {
            to = payable(addr);
        }
        to.transfer(1); //~WARN: local variable is read before being initialized
    }

    // Compound assignment reads x before writing, so x is uninitialized.
    function compoundRead() public pure returns (uint256) {
        uint256 x;
        x += 1; //~WARN: local variable is read before being initialized
        return x;
    }

    // SHOULD NOT WARN:

    // Assigned before read.
    function assignedFirst() public pure returns (uint256) {
        uint256 x;
        x = 5;
        return x;
    }

    // Has an explicit initializer.
    function initialized() public pure returns (uint256) {
        uint256 y = 0;
        return y;
    }

    // Both branches assign before read.
    function bothBranches(bool flag, address a, address b) public {
        address payable to;
        if (flag) {
            to = payable(a);
        } else {
            to = payable(b);
        }
        to.transfer(1);
    }

    // Function parameter, not a statement-declared local.
    function paramUsed(uint256 v) public pure returns (uint256) {
        return v;
    }

    // State variable, not a local.
    function stateUsed() public view returns (address) {
        return owner;
    }

    // Tuple destructuring writes both variables: no warning.
    function tupleWrite() public pure returns (uint256) {
        uint256 a;
        uint256 b;
        (a, b) = foo();
        return a + b;
    }

    // Partial tuple: second slot written, first skipped: no warning on x.
    function tupleSkipFirst() public pure returns (uint256) {
        uint256 x;
        (, x) = foo();
        return x;
    }

    // delete is an explicit write to zero: no warning.
    function deleteWrite() public pure returns (uint256) {
        uint256 x;
        delete x;
        return x;
    }

    // Reference types have well-defined defaults: no warning.
    function memoryArray() public pure returns (uint256) {
        uint256[] memory a;
        return a.length;
    }

    function memoryBytes() public pure returns (uint256) {
        bytes memory b;
        return b.length;
    }

    function memoryString() public pure returns (uint256) {
        string memory s;
        return bytes(s).length;
    }

    function memoryStruct() public pure returns (uint256) {
        Point memory p;
        return p.x;
    }

    // do-while body always executes once: write is guaranteed.
    function doWhileWrite(bool cond) public pure returns (uint256) {
        uint256 x;
        do {
            x = 5;
        } while (cond);
        return x;
    }

    // Only then-branch writes but it always returns: x is initialized on the fall-through path.
    function thenAlwaysReturns(bool flag) public pure returns (uint256) {
        uint256 x;
        if (flag) {
            return 0;
        } else {
            x = 5;
        }
        return x;
    }

    // Only else-branch writes but it always returns: x is initialized on the fall-through path.
    function elseAlwaysReturns(bool flag) public pure returns (uint256) {
        uint256 x;
        if (flag) {
            x = 5;
        } else {
            return 0;
        }
        return x;
    }

    // SHOULD WARN: try/catch — write only in success clause, uninitialized on revert path.
    function tryWrite(address target) public returns (uint256) {
        uint256 x;
        try IFoo(target).getValue() returns (uint256 v) {
            x = v;
        } catch {
            // x not written
        }
        return x; //~WARN: local variable is read before being initialized
    }

    function foo() internal pure returns (uint256, uint256) {
        return (1, 2);
    }
}

interface IFoo {
    function getValue() external returns (uint256);
}
