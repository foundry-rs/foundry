//@compile-flags: --only-lint missing-events-access-control

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract MissingEventsAccessControl {
    address public owner = msg.sender;
    address public pendingOwner;
    address public guardian;
    address public operator;
    address public fixedOwner;
    address public plainAddress;
    address public observedAddress;
    uint256 public threshold;
    mapping(address => bool) public roles;
    mapping(address => mapping(bytes32 => bool)) public namedRoles;
    mapping(bytes32 => mapping(address => bool)) public nestedRoles;

    event OwnershipTransferred(address indexed oldOwner, address indexed newOwner);
    event GuardianUpdated(address indexed guardian);
    event RoleUpdated(address indexed account, bool enabled);
    event Touched();

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaCheck() {
        _checkOwner();
        _;
    }

    modifier onlyRole() {
        require(roles[msg.sender], "missing role");
        _;
    }

    modifier onlyThreshold(uint256 value) {
        require(value > threshold, "too small");
        _;
    }

    // SHOULD FAIL:

    function transferOwnership(address newOwner) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function acceptOwnership() external {
        if (msg.sender != pendingOwner) revert();
        owner = pendingOwner;
        pendingOwner = address(0);
    }

    function setGuardian(address newGuardian) external onlyOwner {
        address nextGuardian = newGuardian;
        guardian = nextGuardian; //~WARN: `guardian` is changed without an event but is used for access control
    }

    function setGuardianInternal(address newGuardian) external onlyOwner {
        _setGuardian(newGuardian);
    }

    function _setGuardian(address newGuardian) internal {
        guardian = newGuardian; //~WARN: `guardian` is changed without an event but is used for access control
    }

    function grantRole(address account) external onlyOwner {
        roles[account] = true; //~WARN: `roles` is changed without an event but is used for access control
    }

    function setNamedRole(address account, bytes32 role, bool enabled) external onlyOwner {
        namedRoles[account][role] = enabled; //~WARN: `namedRoles` is changed without an event but is used for access control
    }

    function grantNestedRole(bytes32 role, address account) external onlyNestedRole(bytes32(0)) {
        _grantNestedRole(role, account);
    }

    function _grantNestedRole(bytes32 role, address account) internal {
        nestedRoles[role][account] = true; //~WARN: `nestedRoles` is changed without an event but is used for access control
    }

    function setOwnerOZStyle(address newOwner) external onlyOwnerViaCheck {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    // Access-control usage that makes the state critical.

    function ownerAction() external view onlyOwner returns (uint256) {
        return 1;
    }

    function pendingOwnerAction() external view {
        if (msg.sender != pendingOwner) revert();
    }

    function guardianAction() external view {
        _checkGuardian();
    }

    function roleAction() external view onlyRole returns (uint256) {
        return 2;
    }

    function namedRoleAction(bytes32 role) external view {
        require(namedRoles[msg.sender][role], "missing role");
    }

    modifier onlyNestedRole(bytes32 role) {
        _checkRole(role);
        _;
    }

    function _checkRole(bytes32 role) internal view {
        _checkRole(role, _msgSender());
    }

    function _checkRole(bytes32 role, address account) internal view {
        if (!hasNestedRole(role, account)) revert();
    }

    function hasNestedRole(bytes32 role, address account) public view returns (bool) {
        return nestedRoles[role][account];
    }

    // SHOULD PASS:

    function transferOwnershipWithEvent(address newOwner) external onlyOwner {
        address oldOwner = owner;
        owner = newOwner;
        emit OwnershipTransferred(oldOwner, newOwner);
    }

    function setGuardianWithInternalEvent(address newGuardian) external onlyOwner {
        _setGuardianWithEvent(newGuardian);
    }

    function _setGuardianWithEvent(address newGuardian) internal {
        guardian = newGuardian;
        emit GuardianUpdated(newGuardian);
    }

    function setWithUnrelatedEvent(address newOwner) external onlyOwner {
        emit Touched();
        owner = newOwner;
    }

    function grantRoleWithEvent(address account) external onlyOwner {
        roles[account] = true;
        emit RoleUpdated(account, true);
    }

    function proposeOwner(address newOwner) external onlyOwner {
        pendingOwner = newOwner;
        emit OwnershipTransferred(owner, newOwner);
    }

    function unprotectedSetOwner(address newOwner) external {
        owner = newOwner;
    }

    function setPlainAddress(address newValue) external onlyOwner {
        plainAddress = newValue;
    }

    function readPlainAddress() external view returns (address) {
        return plainAddress;
    }

    function setFixedOwner() external onlyOwner {
        fixedOwner = address(0xBEEF);
    }

    function fixedOwnerAction() external view {
        require(msg.sender == fixedOwner, "not fixed owner");
    }

    function observesSenderButDoesNotRestrict(address newValue) external {
        if (msg.sender == owner) {
            newValue = address(0);
        }
        observedAddress = newValue;
    }

    function setThreshold(uint256 newThreshold) external onlyOwner {
        threshold = newThreshold;
    }

    function thresholdAction(uint256 value) external view onlyThreshold(value) returns (uint256) {
        return value;
    }

    constructor(address initialOwner) {
        owner = initialOwner;
    }

    function _checkOwner() internal view {
        if (owner != _msgSender()) revert();
    }

    function _checkGuardian() internal view {
        require(_msgSender() == guardian, "not guardian");
    }

    function _msgSender() internal view returns (address) {
        return msg.sender;
    }
}

contract AccessBase {
    address public baseOwner;

    modifier onlyBaseOwner() {
        require(msg.sender == baseOwner, "not base owner");
        _;
    }
}

contract MissingEventsAccessControlDerived is AccessBase {
    function transferBaseOwnership(address newOwner) external onlyBaseOwner {
        baseOwner = newOwner; //~WARN: `baseOwner` is changed without an event but is used for access control
    }
}
