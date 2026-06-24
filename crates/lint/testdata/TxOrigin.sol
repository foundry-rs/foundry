//@compile-flags: --only-lint tx-origin
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract TxOrigin {
    address public owner;
    mapping(address => bool) public allowed;

    constructor() {
        owner = msg.sender;
    }

    modifier onlyOwner() {
        require(tx.origin == owner, "not owner"); //~WARN: `tx.origin` should not be used for authorization
        _;
    }

    function guardedByIf() external view {
        if (tx.origin != owner) { //~WARN: `tx.origin` should not be used for authorization
            revert("not owner");
        }
    }

    function guardedByPredicate() external view {
        assert(isOwner(tx.origin)); //~WARN: `tx.origin` should not be used for authorization
    }

    function guardedByWhile() external view {
        while (tx.origin == owner) { //~WARN: `tx.origin` should not be used for authorization
            break;
        }
    }

    function guardedByFor() external view {
        for (; tx.origin == owner;) { //~WARN: `tx.origin` should not be used for authorization
            break;
        }
    }

    function guardedByDoWhile() external view {
        do {
        } while (tx.origin == owner); //~WARN: `tx.origin` should not be used for authorization
    }

    function guardedByMapping() external view {
        require(allowed[tx.origin], "not allowed"); //~WARN: `tx.origin` should not be used for authorization
        require(allowed[tx.origin] == true, "not allowed"); //~WARN: `tx.origin` should not be used for authorization
    }

    function guardedByTernary() external view {
        require(tx.origin == owner ? true : false, "not owner"); //~WARN: `tx.origin` should not be used for authorization
    }

    function readForLogging() external view returns (address) {
        return tx.origin;
    }

    function explicitSenderCheck() external view {
        require(msg.sender == owner, "not owner");
    }

    function isOwner(address account) internal view returns (bool) {
        return account == owner;
    }
}
