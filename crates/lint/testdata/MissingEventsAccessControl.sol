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
    event Logged(address val);
    event Touched();

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    modifier loggedModifier() {
        emit Touched();
        _;
    }

    modifier onlyOwnerViaSenderAlias() {
        address sender = msg.sender;
        require(sender == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaReassignedAlias(address newOwner) {
        address caller = msg.sender;
        caller = newOwner;
        require(caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaAliasBeforeReassign(address newOwner) {
        address caller = msg.sender;
        require(caller == owner, "not owner");
        caller = newOwner;
        _;
    }

    modifier onlyOwnerViaTupleReassignedAlias(address newOwner) {
        address caller = msg.sender;
        (caller,) = (newOwner, false);
        require(caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaTupleSwappedAlias(address newOwner) {
        address caller = newOwner;
        address previous;
        (caller, previous) = (msg.sender, caller);
        require(previous == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaTupleTransferredAlias(address newOwner) {
        address caller = msg.sender;
        address previous;
        (caller, previous) = (newOwner, caller);
        require(previous == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaTupleRepeatedSenderFirst(address newOwner) {
        address caller;
        (caller, caller) = (msg.sender, newOwner);
        require(caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaTupleRepeatedCandidateFirst(address newOwner) {
        address caller;
        (caller, caller) = (newOwner, msg.sender);
        require(caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaNestedTupleAlias() {
        address caller;
        address spare;
        bool live;
        ((caller, spare), live) = ((msg.sender, address(0)), true);
        require(live && caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaNestedTupleReassignedAlias(address newOwner) {
        address caller = msg.sender;
        address spare;
        bool live;
        ((caller, spare), live) = ((newOwner, address(0)), false);
        require(caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaTupleDeclaredAlias() {
        (address caller, bool live) = (msg.sender, true);
        require(live && caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaTupleCallAlias() {
        (address caller,) = _senderAndFlag();
        require(caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaTupleCallAssignedAlias() {
        address caller;
        (, caller) = _flagAndSender();
        require(caller == owner, "not owner");
        _;
    }

    modifier onlyOwnerViaTupleCallReassignedAlias(address newOwner) {
        address caller = msg.sender;
        (, caller) = _flagAndAddress(newOwner);
        require(caller == owner, "not owner");
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

    modifier guardAfterPlaceholder() {
        _;
        require(msg.sender == owner, "not owner");
    }

    modifier writesOwner(address newOwner) {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        _;
    }

    // SHOULD FAIL:

    function transferOwnership(address newOwner) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function acceptOwnership() external {
        if (msg.sender != pendingOwner) revert();
        owner = pendingOwner; //~WARN: `owner` is changed without an event but is used for access control
        pendingOwner = address(0); //~WARN: `pendingOwner` is changed without an event but is used for access control
    }

    function acceptOwnershipFromSender() external {
        if (msg.sender != pendingOwner) revert();
        owner = msg.sender; //~WARN: `owner` is changed without an event but is used for access control
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

    function revokeRole(address account) external onlyOwner {
        delete roles[account]; //~WARN: `roles` is changed without an event but is used for access control
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

    function setOwnerWithUnrelatedEventBefore(address newOwner) external onlyOwner {
        emit Touched();
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerWithUnrelatedEventAfter(address newOwner) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        emit Touched();
    }

    function setOwnerWithUnrelatedArgEvent(address newOwner) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        emit Logged(newOwner);
    }

    function setOwnerViaLoggingModifier(address newOwner) external onlyOwner loggedModifier {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerEventInOtherBranch(address newOwner) external onlyOwner {
        if (newOwner == address(0)) {
            emit Touched();
        }
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaSenderAlias(address newOwner) external onlyOwnerViaSenderAlias {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaReassignedAlias(address newOwner)
        external
        onlyOwnerViaReassignedAlias(newOwner)
    {
        owner = newOwner;
    }

    function setOwnerViaAliasBeforeReassign(address newOwner)
        external
        onlyOwnerViaAliasBeforeReassign(newOwner)
    {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaTupleReassignedAlias(address newOwner)
        external
        onlyOwnerViaTupleReassignedAlias(newOwner)
    {
        owner = newOwner;
    }

    function setOwnerViaTupleSwappedAlias(address newOwner)
        external
        onlyOwnerViaTupleSwappedAlias(newOwner)
    {
        owner = newOwner;
    }

    function setOwnerViaTupleTransferredAlias(address newOwner)
        external
        onlyOwnerViaTupleTransferredAlias(newOwner)
    {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaTupleRepeatedSenderFirst(address newOwner)
        external
        onlyOwnerViaTupleRepeatedSenderFirst(newOwner)
    {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaTupleRepeatedCandidateFirst(address newOwner)
        external
        onlyOwnerViaTupleRepeatedCandidateFirst(newOwner)
    {
        owner = newOwner;
    }

    function setOwnerViaNestedTupleAlias(address newOwner) external onlyOwnerViaNestedTupleAlias {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaNestedTupleReassignedAlias(address newOwner)
        external
        onlyOwnerViaNestedTupleReassignedAlias(newOwner)
    {
        owner = newOwner;
    }

    function setOwnerViaTupleDeclaredAlias(address newOwner)
        external
        onlyOwnerViaTupleDeclaredAlias
    {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaTupleCallAlias(address newOwner) external onlyOwnerViaTupleCallAlias {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaTupleCallAssignedAlias(address newOwner)
        external
        onlyOwnerViaTupleCallAssignedAlias
    {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function setOwnerViaTupleCallReassignedAlias(address newOwner)
        external
        onlyOwnerViaTupleCallReassignedAlias(newOwner)
    {
        owner = newOwner;
    }

    function setOwnerInModifier(address newOwner) external onlyOwner writesOwner(newOwner) {}

    function grantRoleViaStorageAlias(address account) external onlyOwner {
        mapping(address => bool) storage roleSet = roles;
        roleSet[account] = true; //~WARN: `roles` is changed without an event but is used for access control
    }

    function setNamedRoleViaStorageAlias(
        address account,
        bytes32 role,
        bool enabled
    ) external onlyOwner {
        mapping(bytes32 => bool) storage accountRoles = namedRoles[account];
        accountRoles[role] = enabled; //~WARN: `namedRoles` is changed without an event but is used for access control
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

    function grantRoleWithEvent(address account) external onlyOwner {
        roles[account] = true;
        emit RoleUpdated(account, true);
    }

    function proposeOwner(address newOwner) external onlyOwner {
        pendingOwner = newOwner;
        emit OwnershipTransferred(owner, newOwner);
    }

    function renounceOwnershipWithEvent() external onlyOwner {
        address oldOwner = owner;
        owner = address(0);
        emit OwnershipTransferred(oldOwner, address(0));
    }

    function unprotectedSetOwner(address newOwner) external {
        owner = newOwner;
    }

    function setOwnerWithComputedSender(address expected, address newOwner) external {
        require(msg.sender == computeAddress(expected), "not computed");
        owner = newOwner;
    }

    function setOwnerViaOverwrittenLocal(address newOwner) external onlyOwner {
        address next = newOwner;
        next = address(0xBEEF);
        owner = next;
    }

    function setOwnerAfterRevertingTaint(address newOwner) external onlyOwner {
        address next = address(0xBEEF);
        if (newOwner == address(0)) {
            next = newOwner;
            revert();
        }
        owner = next;
    }

    function setOwnerGuardAfterPlaceholder(address newOwner) external guardAfterPlaceholder {
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

    function senderAliasCycleEntry() external view {
        senderAliasCycleA();
    }

    function senderAliasCycleA() internal view returns (address) {
        address sender = senderAliasCycleB();
        return sender;
    }

    function senderAliasCycleB() internal view returns (address) {
        address sender = senderAliasCycleA();
        return sender;
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

    function _senderAndFlag() internal view returns (address, bool) {
        return (msg.sender, true);
    }

    function _flagAndSender() internal view returns (bool, address) {
        return (true, msg.sender);
    }

    function _flagAndAddress(address value) internal pure returns (bool, address) {
        return (true, value);
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

    function computeAddress(address expected) internal pure returns (address) {
        return expected;
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

abstract contract AbstractAccessBase {
    address public admin;

    modifier onlyAdmin() virtual;
}

abstract contract MissingEventsAccessControlAbstractDerived is AbstractAccessBase {
    function setAdmin(address newAdmin) external onlyAdmin {
        admin = newAdmin; //~WARN: `admin` is changed without an event but is used for access control
    }

    function adminAction() external view {
        require(msg.sender == admin, "not admin");
    }
}

abstract contract MissingEventsAccessControlAssertionHelpers {
    bool private _failed;

    function assertEq(uint128 left, uint128 right) internal view virtual {
        assertEq(uint256(left), uint256(right));
    }

    function assertEq(uint256 left, uint256 right) internal view virtual {
        if (left != right || _failed) {
            fail();
        }
    }

    function fail() internal view virtual {
        assertEq(uint256(0), uint256(1));
    }
}

contract NamedArgumentAccessControl {
    address public owner = msg.sender;

    function setOwner(address next) external {
        require(msg.sender == owner);
        _setOwner({next: next, ignored: address(1)});
    }

    function _setOwner(address ignored, address next) internal {
        owner = next; //~WARN: `owner` is changed without an event but is used for access control
    }
}

contract MissingEventsAccessControlForgeStdLikeTest is MissingEventsAccessControlAssertionHelpers {
    function testFuzz_SendMail(uint128 mintAmount, uint128 sendAmount) public view {
        assertEq(uint256(mintAmount), uint256(sendAmount));
    }
}
