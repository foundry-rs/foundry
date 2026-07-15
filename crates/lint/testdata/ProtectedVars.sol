//@compile-flags: --only-lint protected-vars

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract DirectWrites {
    /// @custom:security write-protection="onlyOwner()"
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function setOwner(address newOwner) external { //~WARN: protected variable `owner` is written without `onlyOwner()`
        owner = newOwner;
    }

    function setOwnerProtected(address newOwner) external onlyOwner {
        owner = newOwner;
    }
}

contract InternalCalls {
    /// @custom:security write-protection="checkOwner()"
    address public owner;

    function checkOwner() internal view {
        require(msg.sender == owner);
    }

    function setThroughHelper(address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        writeOwner(newOwner);
    }

    function setThroughProtectedHelper(address newOwner) external {
        protectedWriteOwner(newOwner);
    }

    function protectedWriteOwner(address newOwner) internal {
        checkOwner();
        writeOwner(newOwner);
    }

    function writeOwner(address newOwner) internal {
        owner = newOwner;
    }
}

contract ModifierPaths {
    /// @custom:security write-protection="onlyOwner()"
    address public owner;

    modifier onlyOwner() {
        _checkOwner();
        _;
    }

    modifier writeOwner(address newOwner) {
        owner = newOwner;
        _;
    }

    function _checkOwner() internal view {
        require(msg.sender == owner);
    }

    function writeInUnprotectedModifier(address newOwner) //~WARN: protected variable `owner` is written without `onlyOwner()`
        external
        writeOwner(newOwner)
    {}

    function writeInProtectedHelper(address newOwner) external {
        _writeInProtectedHelper(newOwner);
    }

    function _writeInProtectedHelper(address newOwner) internal onlyOwner {
        owner = newOwner;
    }
}

contract FunctionFromModifier {
    /// @custom:security write-protection="checkOwner()"
    address public owner;

    modifier checked() {
        checkOwner();
        _;
    }

    function checkOwner() internal view {
        require(msg.sender == owner);
    }

    function setOwner(address newOwner) external checked {
        owner = newOwner;
    }
}

contract ExactSignatures {
    /// @custom:security write-protection="onlyRole(bytes32)"
    bytes32 public role;

    modifier onlyRole(bytes32 expected) {
        require(role == expected);
        _;
    }

    function setRole(bytes32 oldRole, bytes32 newRole) external onlyRole(oldRole) {
        role = newRole;
    }

    function setRoleUnprotected(bytes32 newRole) external { //~WARN: protected variable `role` is written without `onlyRole(bytes32)`
        role = newRole;
    }
}

contract ExternalGuardCall {
    /// @custom:security write-protection="checkOwner()"
    address public owner;

    function checkOwner() external view {
        require(msg.sender == owner);
    }

    function setOwner(address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        this.checkOwner();
        owner = newOwner;
    }
}

contract CollectionWrites {
    /// @custom:security write-protection="onlyOwner()"
    address[] public members;

    /// @custom:security write-protection="onlyOwner()"
    mapping(address => bool) public allowed;

    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function addMember(address member) external { //~WARN: protected variable `members` is written without `onlyOwner()`
        members.push(member);
    }

    function clearMember(address member) external { //~WARN: protected variable `allowed` is written without `onlyOwner()`
        delete allowed[member];
    }

    function removeMember() external onlyOwner {
        members.pop();
    }
}

contract StorageAliases {
    struct Settings {
        address operator;
    }

    /// @custom:security write-protection="onlyOwner()"
    Settings internal settings;

    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function setOperator(address operator) external { //~WARN: protected variable `settings` is written without `onlyOwner()`
        Settings storage current = settings;
        current.operator = operator;
    }

    function setOperatorThroughParameter(address operator) external { //~WARN: protected variable `settings` is written without `onlyOwner()`
        writeSettings(settings, operator);
    }

    function writeSettings(Settings storage current, address operator) internal {
        current.operator = operator;
    }
}

contract ReachabilitySemantics {
    /// @custom:security write-protection="checkOwner()"
    address public owner;

    function checkOwner() internal view {
        require(msg.sender == owner);
    }

    // Slither's annotation is reachability-based, so the call need not dominate the write.
    function branchGuard(bool guard, address newOwner) external {
        if (guard) checkOwner();
        owner = newOwner;
    }

    function guardAfterWrite(address newOwner) external {
        owner = newOwner;
        checkOwner();
    }
}

contract BaseProtection {
    /// @custom:security write-protection="onlyOwner()"
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function inheritedUnsafe(address newOwner) public { //~WARN: protected variable `owner` is written without `onlyOwner()`
        owner = newOwner;
    }

    function inheritedSafe(address newOwner) public onlyOwner {
        owner = newOwner;
    }
}

contract DerivedProtection is BaseProtection {
    function derivedUnsafe(address newOwner) external { //~WARN: protected variable `owner` is written without `onlyOwner()`
        owner = newOwner;
    }

    function derivedSafe(address newOwner) external onlyOwner {
        owner = newOwner;
    }
}

contract UnresolvedProtection {
    /// @custom:security write-protection="notDeclared()"
    address public owner;

    // Invalid requirements fail closed instead of silently disabling the security annotation.
    function setOwner(address newOwner) external { //~WARN: protected variable `owner` is written without `notDeclared()`
        owner = newOwner;
    }
}

contract MalformedProtection {
    /// @custom:security write-protection="onlyOwner()
    address public owner;

    /// @custom:security no-write-protection
    address public unprotected;

    function setOwner(address newOwner) external { //~WARN: protected variable `owner` has a malformed write-protection annotation
        owner = newOwner;
    }

    function setUnprotected(address newValue) external {
        unprotected = newValue;
    }
}
