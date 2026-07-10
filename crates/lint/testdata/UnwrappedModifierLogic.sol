//@compile-flags: --severity code-size

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

    // Bad because there are multiple valid function calls before the placeholder.
    // The single call after the placeholder must be preserved in the rewrite.
    modifier beforeWrappedAfterKept(address sender) { //~NOTE: wrap modifier logic to reduce code size
        checkPublic(sender); // These should become _beforeWrappedAfterKept(sender)
        checkPrivate(sender);
        _;
        checkInternal(sender); // This should stay in the modifier
    }

    // Bad because there are multiple valid function calls after the placeholder.
    // The single call before the placeholder must be preserved in the rewrite.
    modifier afterWrappedBeforeKept(address sender) { //~NOTE: wrap modifier logic to reduce code size
        checkPublic(sender); // This should stay in the modifier
        _;
        checkPrivate(sender); // These should become _afterWrappedBeforeKept(sender)
        checkInternal(sender);
    }

    // Bad because there are multiple valid function calls after the placeholder.
    // The assembly block before the placeholder must be preserved in the rewrite.
    modifier keepAssemblyBefore() { //~NOTE: wrap modifier logic to reduce code size
        assembly ("memory-safe") {
            mstore(0x00, 0)
        }
        _;
        checkPublic(msg.sender); // These should become _keepAssemblyBefore()
        checkPrivate(msg.sender);
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

    /// -----------------------------------------------------------------------
    /// Exceptions (locals declared before the placeholder and used after it)
    /// -----------------------------------------------------------------------

    function gasLeft() internal view returns (uint256) {
        return gasleft();
    }

    function _payMeSubsidizedGasAfter(uint256 pre, uint256 amount) internal {}

    // No lint: `pre` cannot be forwarded to the extracted helper.
    modifier payMeSubsidizedGas(uint256 amount) {
        uint256 pre = gasLeft();
        _;
        _payMeSubsidizedGasAfter(pre, amount);
    }

    // No lint: `pre` is shared across the placeholder, even though both sides would
    // otherwise be wrapped.
    modifier payMeSubsidizedGasAndLog(uint256 amount) {
        uint256 pre = gasLeft();
        checkPublic(msg.sender);
        _;
        checkPrivate(msg.sender);
        _payMeSubsidizedGasAfter(pre, amount);
    }

    // No lint: `pre` is used after the placeholder, and the assembly block prevents
    // wrapping the declaring side.
    modifier trackFreeMemory() {
        uint256 pre;
        assembly ("memory-safe") {
            pre := mload(0x40)
        }
        _;
        checkPublic(msg.sender);
        require(pre != 0, "no free memory");
    }

    // No lint: `m` is used inside the assembly block after the placeholder.
    modifier restoreFreeMemory() {
        uint256 m = gasLeft();
        checkPublic(msg.sender);
        _;
        assembly ("memory-safe") {
            mstore(0x40, m)
        }
    }
}
