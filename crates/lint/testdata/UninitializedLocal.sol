//@compile-flags: --only-lint uninitialized-local

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract UninitializedLocal {
    address public owner;

    // SHOULD WARN:

    // Classic slither example: `to` is never assigned, defaults to address(0).
    function withdraw() public {
        address payable to; //~WARN: local variable is read before being initialized
        to.transfer(address(this).balance);
    }

    // Returning an uninitialized uint silently returns 0.
    function getAmount() public pure returns (uint256) {
        uint256 amount; //~WARN: local variable is read before being initialized
        return amount;
    }

    // Reading an uninitialized var in an expression.
    function compute(uint256 b) public pure returns (uint256) {
        uint256 a; //~WARN: local variable is read before being initialized
        return a + b;
    }

    // Reading an uninitialized var in the initializer of another local.
    function alias_() public pure returns (address) {
        address to; //~WARN: local variable is read before being initialized
        address from = to;
        return from;
    }

    // Only one branch assigns, so still uninitialized on the path that skips the branch.
    function conditional(bool flag, address addr) public {
        address payable to; //~WARN: local variable is read before being initialized
        if (flag) {
            to = payable(addr);
        }
        to.transfer(1);
    }

    // Compound assignment reads x before writing, so x is uninitialized.
    function compoundRead() public pure returns (uint256) {
        uint256 x; //~WARN: local variable is read before being initialized
        x += 1;
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
}
