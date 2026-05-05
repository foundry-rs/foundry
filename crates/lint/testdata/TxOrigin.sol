//@compile-flags: --only-lint tx-origin
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract TxOrigin {
    address public owner;

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
