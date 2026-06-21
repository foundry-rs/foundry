//@compile-flags: --only-lint non-reentrant-not-first

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract NonReentrantNotFirst {
    modifier nonReentrant() {
        _;
    }

    modifier onlyOwner() {
        _;
    }

    modifier whenNotPaused() {
        _;
    }

    function badSingle(uint256 amount) external onlyOwner nonReentrant { //~WARN: `nonReentrant` should be the first modifier
        amount;
    }

    function badMultiple(uint256 amount) external onlyOwner whenNotPaused nonReentrant { //~WARN: `nonReentrant` should be the first modifier
        amount;
    }

    function goodFirst(uint256 amount) external nonReentrant onlyOwner {
        amount;
    }

    function goodOnly(uint256 amount) external nonReentrant {
        amount;
    }

    function goodNoGuard(uint256 amount) external onlyOwner whenNotPaused {
        amount;
    }

    receive() external payable nonReentrant onlyOwner {}
}

contract BaseReentrancyGuard {
    modifier nonReentrant() {
        _;
    }
}

contract InheritedNonReentrantNotFirst is BaseReentrancyGuard {
    modifier onlyOwner() {
        _;
    }

    function badInherited() external onlyOwner nonReentrant { //~WARN: `nonReentrant` should be the first modifier
        msg.sender;
    }

    function goodInherited() external nonReentrant onlyOwner {
        msg.sender;
    }
}
