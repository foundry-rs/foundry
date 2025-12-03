// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

library Lib {
    function onlyOwner(address sender) internal {}
}

contract C {
    function onlyOwner(address sender) public {}
}

/**
 * @title UnwrappedModifierLogicTest
 * @notice Test cases for the unwrapped-modifier-logic lint
 * @dev This lint helps optimize gas by preventing modifier code duplication.
 *      Solidity inlines modifier code at each usage point instead of using jumps,
 *      so any logic in modifiers gets duplicated, increasing deployment costs.
 */
contract UnwrappedModifierLogicTest {
    // Helpers

    C immutable c;

    event DidSomething(address who);
    mapping(address => bool) isOwner;
    mapping(address => mapping(bytes32 => bool)) hasRole;

    /// -----------------------------------------------------------------------
    /// Exceptions (assembly block)
    /// -----------------------------------------------------------------------

    modifier freeTempMemory() {
        uint256 m;
        assembly ("memory-safe") {
            m := mload(0x40)
        }
        _;
        assembly ("memory-safe") {
            mstore(0x40, m)
        }
    }

    modifier assemblyBlock(address sender) {
        assembly {
            let x := sender
        }
        _;
    }

    /// -----------------------------------------------------------------------
    /// Good patterns (only 1 valid statement before or after placeholder)
    /// -----------------------------------------------------------------------

    function checkPublic(address sender) public {}
    function checkPrivate(address sender) private {}
    function checkInternal(address sender) internal {}

    modifier onlyOwnerLibrary() {
        Lib.onlyOwner(msg.sender);
        _;
    }

    modifier onlyOwnerPublic() {
        checkPublic(msg.sender);
        _;
    }

    modifier onlyOwnerPrivate() {
        checkPrivate(msg.sender);
        _;
    }

    modifier onlyOwnerInternal() {
        checkInternal(msg.sender);
        _;
    }

    modifier onlyOwnerBeforeAfter() {
        checkPublic(msg.sender);
        _;
        checkPrivate(msg.sender);
    }

    /// -----------------------------------------------------------------------
    /// Shared local variables
    /// -----------------------------------------------------------------------

    function gasLeft() internal returns (uint256) { return 1; }
    function gasLeftMulti() internal returns (uint256, bool) { return (1, true); }
    function _payMeSubsidizedGasAfter(uint256, uint256) internal {}
    function _refund(uint256) internal {}

    // Single shared variable: declared before, used after
    modifier payMeSubsidizedGas(uint256 amount) {
        uint256 pre = gasLeft();
        _;
        _payMeSubsidizedGasAfter(pre, amount);
    }

    // Multiple shared variables
    modifier payMeFixedGasAmount() { //~NOTE: wrap modifier logic to reduce code size
        uint256 pre = gasLeft();
        uint256 amount = 12345;
        _;
        _payMeSubsidizedGasAfter(pre, amount);
    }

    modifier payMeSubsidizedGasAndRefund(uint256 amount) { //~NOTE: wrap modifier logic to reduce code size
        (uint256 pre, bool success) = gasLeftMulti();
        _;
        _payMeSubsidizedGasAfter(pre, amount);
        _refund(pre);
    }

    // Multiple shared variables
    modifier payMeFixedGasAmountAndRefund() { //~NOTE: wrap modifier logic to reduce code size
        uint256 pre = gasLeft();
        uint256 amount = 12345;
        _;
        _payMeSubsidizedGasAfter(pre, amount);
        _refund(pre);
    }

    /// -----------------------------------------------------------------------
    /// Bad patterns (multiple valid statements before or after placeholder)
    /// -----------------------------------------------------------------------

    // Bad because there are multiple valid function calls before the placeholder
    modifier multipleBeforePlaceholder() { //~NOTE: wrap modifier logic to reduce code size
        checkPublic(msg.sender); // These should become _multipleBeforePlaceholder()
        checkPrivate(msg.sender);
        checkInternal(msg.sender);
        _;
    }

    // Bad because there are multiple valid function calls after the placeholder
    modifier multipleAfterPlaceholder() { //~NOTE: wrap modifier logic to reduce code size
        _;
        checkPublic(msg.sender); // These should become _multipleAfterPlaceholder()
        checkPrivate(msg.sender);
        checkInternal(msg.sender);
    }

    // Bad because there are multiple valid statements both before and after
    modifier multipleBeforeAfterPlaceholder(address sender) { //~NOTE: wrap modifier logic to reduce code size
        checkPublic(sender); // These should become _multipleBeforeAfterPlaceholderBefore(sender)
        checkPrivate(sender);
        _;
        checkInternal(sender); // These should become _multipleBeforeAfterPlaceholderAfter(sender)
        checkPublic(sender);
    }

    /// -----------------------------------------------------------------------
    /// Bad patterns (uses built-in control flow)
    /// -----------------------------------------------------------------------

    // Bad because `require` built-in is used.
    modifier onlyOwner() { //~NOTE: wrap modifier logic to reduce code size
        require(isOwner[msg.sender], "Not owner"); // _onlyOwner();
        _;
    }

    // Bad because `if/revert` is used.
    modifier onlyRole(bytes32 role) { //~NOTE: wrap modifier logic to reduce code size
        if(!hasRole[msg.sender][role]) revert("Not authorized"); // _onlyRole(role);
        _;
    }

    // Bad because `assert` built-in is used.
    modifier onlyRoleOrOpenRole(bytes32 role) { //~NOTE: wrap modifier logic to reduce code size
        assert(hasRole[msg.sender][role] || hasRole[address(0)][role]); // _onlyRoleOrOpenRole(role);
        _;
    }

    // Bad because `assert` built-in is used (ensures we can parse multiple params).
    modifier onlyRoleOrAdmin(bytes32 role, address admin) { //~NOTE: wrap modifier logic to reduce code size
        assert(hasRole[msg.sender][role] || msg.sender == admin); // _onlyRoleOrAdmin(role, admin);
        _;
    }

    /// -----------------------------------------------------------------------
    /// Bad patterns (other invalid expressions and statements)
    /// -----------------------------------------------------------------------

    // Only call expressions are allowed (public/private/internal functions).
    modifier assign(address sender) { //~NOTE: wrap modifier logic to reduce code size
        bool _isOwner = true;
        isOwner[sender] = _isOwner;
        _;
    }

    // Only call expressions are allowed (public/private/internal functions).
    modifier uncheckedBlock(address sender) { //~NOTE: wrap modifier logic to reduce code size
        unchecked {
            sender;
        }
        _;
    }

    // Only call expressions are allowed (public/private/internal functions).
    modifier emitEvent(address sender) { //~NOTE: wrap modifier logic to reduce code size
        emit DidSomething(sender);
        _;
    }

    /// -----------------------------------------------------------------------
    /// Bad patterns (contract calls)
    /// -----------------------------------------------------------------------

    // Bad because there's an external call.
    modifier onlyOwnerContract(address sender) { //~NOTE: wrap modifier logic to reduce code size
        c.onlyOwner(sender);
        _;
    }
}
