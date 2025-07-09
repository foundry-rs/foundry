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

    // Good patterns: Only call internal/private/public methods

    modifier empty() {
        _;
    }

    modifier onlyOwnerPublic() {
        checkOwnerPublic(msg.sender);
        _;
    }

    modifier onlyOwnerPrivate() {
        checkOwnerPrivate(msg.sender);
        _;
    }

    modifier onlyOwnerInternal() {
        checkOwnerInternal(msg.sender);
        _;
    }

    modifier ownerOwnerPublicPrivateInternal(address owner0, address owner1, address owner2) {
        checkOwnerPublic(owner0);
        checkOwnerPrivate(owner1);
        checkOwnerInternal(owner2);
        _;
    }

    modifier singleInternalWithParam(address sender) {
        checkOwnerInternal(sender);
        _;
    }

    modifier multipleInternalWithParam(address owner0, address owner1, address owner2) {
        checkOwnerPublic(owner0);
        checkOwnerPrivate(owner1);
        checkOwnerInternal(owner2);
        _;
    }

    function checkOwnerPublic(address sender) public view {
        require(isOwner[sender], "Not owner");
    }

    function checkOwnerPrivate(address sender) private view {
        require(isOwner[sender], "Not owner");
    }

    function checkOwnerInternal(address sender) internal view {
        require(isOwner[sender], "Not owner");
    }

    // Bad patterns: Any logic that is not just a call to an internal/private/public method

    // 1. require
    modifier onlyOwnerRequire() { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        require(isOwner[msg.sender], "Not owner");
        _;
    }

    // 2. require with param
    modifier onlyOwnerRequireWithParam(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        require(isOwner[sender], "Not owner");
        _;
    }

    // 3. assert
    modifier onlyOwnerAssert() { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        assert(isOwner[msg.sender]);
        _;
    }

    // 4. assert with param
    modifier onlyOwnerAssertWithParam(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        assert(isOwner[sender]);
        _;
    }

    // 5. conditional revert
    modifier onlyOwnerConditionalRevert() { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        if (!isOwner[msg.sender]) {
            revert("Not owner");
        }
        _;
    }

    // 6. conditional revert with param
    modifier onlyOwnerConditionalRevertWithParam(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        if (!isOwner[sender]) {
            revert("Not owner");
        }
        _;
    }

    // 7. assignment
    modifier setOwner(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        isOwner[sender] = true;
        _;
    }

    // 8. assignment with param
    modifier setOwnerWithParam(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        isOwner[sender] = true;
        _;
    }

    // 9. combination: require + internal call
    modifier requireAndInternal(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        require(isOwner[sender], "Not owner");
        checkOwnerInternal(sender);
        _;
    }

    // 10. combination: assignment + internal call
    modifier assignAndInternal(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        isOwner[sender] = true;
        checkOwnerInternal(sender);
        _;
    }

    // 11. combination: require + assignment
    modifier requireAndAssign(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        require(isOwner[sender], "Not owner");
        isOwner[sender] = false;
        _;
    }

    // 12. combination: require + public call
    modifier requireAndPublic(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        require(isOwner[sender], "Not owner");
        checkOwnerPublic(sender);
        _;
    }

    // 13. combination: assignment + public call
    modifier assignAndPublic(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        isOwner[sender] = true;
        checkOwnerPublic(sender);
        _;
    }

    // 14. combination: require + assignment + internal call
    modifier requireAssignInternal(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        require(isOwner[sender], "Not owner");
        isOwner[sender] = false;
        checkOwnerInternal(sender);
        _;
    }

    // 15. combination: require + assignment + public call
    modifier requireAssignPublic(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        require(isOwner[sender], "Not owner");
        isOwner[sender] = false;
        checkOwnerPublic(sender);
        _;
    }

    // 16. inline assembly
    modifier withAssembly(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        assembly {
            let x := sender
        }
        _;
    }

    // 17. event emission
    event DidSomething(address who);

    modifier emitEvent(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        emit DidSomething(sender);
        _;
    }

    // 18. inline revert string
    modifier inlineRevert(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        if (sender == address(0)) revert("Zero address");
        _;
    }

    // 19. combination: event + require + internal call
    modifier eventRequireInternal(address sender) { //~NOTE: modifier logic should be wrapped to avoid code duplication and reduce codesize
        emit DidSomething(sender);
        require(isOwner[sender], "Not owner");
        checkOwnerInternal(sender);
        _;
    }
}