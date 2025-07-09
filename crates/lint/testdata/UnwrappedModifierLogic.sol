// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/**
 * @title UnwrappedModifierLogicTest
 * @notice Test cases for the unwrapped-modifier-logic lint
 * @dev This lint helps optimize gas by preventing modifier code duplication.
 *      Solidity inlines modifier code at each usage point instead of using jumps,
 *      so any logic in modifiers gets duplicated, increasing deployment costs.
 */
contract UnwrappedModifierLogicTest {
    mapping(address => bool) public isOwner;

    // Helpers

    function checkPublic(address sender) public {
        require(isOwner[sender], "Not owner");
    }

    function checkPrivate(address sender) private {
        require(isOwner[sender], "Not owner");
    }

    function checkInternal(address sender) internal {
        require(isOwner[sender], "Not owner");
    }

    // Good patterns

    modifier empty() {
        _;
    }

    modifier publicFn() {
        checkPublic(msg.sender);
        _;
    }

    modifier privateFn() {
        checkPrivate(msg.sender);
        _;
    }

    modifier internalFn() {
        checkInternal(msg.sender);
        _;
    }

    modifier publicPrivateInternal(address owner0, address owner1, address owner2) {
        checkPublic(owner0);
        checkPrivate(owner1);
        checkInternal(owner2);
        _;
    }

    // Bad patterns

    modifier requireBuiltIn() { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        checkPublic(msg.sender);
        require(isOwner[msg.sender], "Not owner");
        checkPrivate(msg.sender);
        _;
        checkInternal(msg.sender);
    }

    modifier assertBuiltIn() { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        checkPublic(msg.sender);
        assert(isOwner[msg.sender]);
        checkPrivate(msg.sender);
        _;
        checkInternal(msg.sender);
    }

    modifier conditionalRevert() { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        checkPublic(msg.sender);
        if (!isOwner[msg.sender]) {
            revert("Not owner");
        }
        checkPrivate(msg.sender);
        _;
        checkInternal(msg.sender);
    }

    modifier assign(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        checkPublic(sender);
        bool _isOwner = true;
        checkPrivate(sender);
        isOwner[sender] = _isOwner;
        _;
        checkInternal(sender);
    }

    modifier assemblyBlock(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        checkPublic(sender);
        assembly {
            let x := sender
        }
        checkPrivate(sender);
        _;
        checkInternal(sender);
    }

    modifier uncheckedBlock(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        checkPublic(sender);
        unchecked {
            sender;
        }
        checkPrivate(sender);
        _;
        checkInternal(sender);
    }

    event DidSomething(address who);

    modifier emitEvent(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        checkPublic(sender);
        emit DidSomething(sender);
        checkPrivate(sender);
        _;
        checkInternal(sender);
    }
}