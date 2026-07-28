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

    /// @custom:security write-protection="checkOwner()"
    mapping(address => address) public owners;

    function checkOwner() internal view {
        require(msg.sender == owner);
    }

    function branchGuard(bool guard, address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        if (guard) checkOwner();
        owner = newOwner;
    }

    function guardAfterWrite(address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        owner = newOwner;
        checkOwner();
    }

    function guardBeforeWrite(address newOwner) external {
        checkOwner();
        owner = newOwner;
    }

    function guardOnEveryBranch(bool branch, address newOwner) external {
        if (branch) checkOwner();
        else checkOwner();
        owner = newOwner;
    }

    function guardOnEveryTryClause(address newOwner) external {
        try this.externalCall() {
            checkOwner();
        } catch {
            checkOwner();
        }
        owner = newOwner;
    }

    function guardOnOneReturnPath(bool skip, address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        maybeGuard(skip);
        owner = newOwner;
    }

    function ternaryGuard(bool guard, address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        guard ? checkOwnerBool() : true;
        owner = newOwner;
    }

    function shortCircuitGuard(bool guard, address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        guard && checkOwnerBool();
        owner = newOwner;
    }

    function guardOnEveryTernaryArm(bool guard, address newOwner) external {
        guard ? checkOwnerBool() : checkOwnerBool();
        owner = newOwner;
    }

    function writeAfterRevertingHelper(address newOwner) external {
        alwaysReverts();
        owner = newOwner;
    }

    function writeAfterRecursiveHelper(address newOwner) external {
        recursiveLoop();
        owner = newOwner;
    }

    function deleteWithRevertingIndex() external {
        delete owners[revertingAddress()];
    }

    function writeAfterRevertingDoWhile(bool repeat, address newOwner) external {
        do {
            alwaysReverts();
        } while (repeat);
        owner = newOwner;
    }

    function writeAfterRevertingWhileCondition(address newOwner) external {
        while (revertingBool()) {}
        owner = newOwner;
    }

    function writeAfterMutualRecursion(address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        recursiveB();
        recursiveB();
        owner = newOwner;
    }

    function writeAfterConditionalRecursion(bool stop, address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        recursiveMaybeWrite(stop, newOwner);
    }

    function writeAfterDoWhileContinue(bool repeat, address newOwner) external { //~WARN: protected variable `owner` is written without `checkOwner()`
        do {
            continue;
        } while (repeat);
        owner = newOwner;
    }

    function maybeGuard(bool skip) internal view {
        if (skip) return;
        checkOwner();
    }

    function checkOwnerBool() internal view returns (bool) {
        checkOwner();
        return true;
    }

    function alwaysReverts() internal pure {
        revert();
    }

    function recursiveLoop() internal pure {
        recursiveLoop();
    }

    function revertingAddress() internal pure returns (address) {
        revert();
    }

    function revertingBool() internal pure returns (bool) {
        revert();
    }

    function recursiveA(bool stop) internal pure {
        if (stop) return;
        recursiveB();
    }

    function recursiveB() internal pure {
        recursiveA(true);
    }

    function recursiveMaybeWrite(bool stop, address newOwner) internal {
        if (stop) return;
        recursiveMaybeWrite(true, newOwner);
        owner = newOwner;
    }

    function externalCall() external pure {}
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

contract ModifierContinuations {
    /// @custom:security write-protection="checkOwner()"
    address public owner;

    modifier blocked() {
        revert();
        _;
    }

    modifier passthrough() {
        _;
    }

    modifier checkAfter() {
        _;
        checkOwner();
    }

    modifier writeAfter(address newOwner) {
        _;
        owner = newOwner;
    }

    modifier revertAfter() {
        _;
        revert();
    }

    function checkOwner() internal view {
        require(msg.sender == owner);
    }

    function unreachableWrite(address newOwner) external blocked {
        owner = newOwner;
    }

    function reachableWrite(address newOwner) external passthrough { //~WARN: protected variable `owner` is written without `checkOwner()`
        owner = newOwner;
    }

    function guardAfterBody(address newOwner) external checkAfter { //~WARN: protected variable `owner` is written without `checkOwner()`
        owner = newOwner;
    }

    function unreachableOuterPost(address newOwner) external writeAfter(newOwner) blocked {}

    function reachableOuterPost(address newOwner) external writeAfter(newOwner) passthrough {} //~WARN: protected variable `owner` is written without `checkOwner()`

    function returnBeforeOuterPost(address newOwner) external writeAfter(newOwner) { //~WARN: protected variable `owner` is written without `checkOwner()`
        return;
    }

    function conditionalReturnBeforeOuterPost(bool skip, address newOwner) //~WARN: protected variable `owner` is written without `checkOwner()`
        external
        writeAfter(newOwner)
    {
        if (skip) return;
        checkOwner();
    }

    function returningHelper() internal revertAfter {
        return;
    }

    function unreachableAfterReturningHelper(address newOwner) external {
        returningHelper();
        owner = newOwner;
    }
}

contract YulStoragePointerRetargeting {
    /// @custom:security write-protection="onlyOwner()"
    uint256[] internal protectedValues;

    uint256[] internal ordinaryValues;

    function retargetAwayFromProtected() external {
        uint256[] storage pointer = protectedValues;
        assembly {
            pointer.slot := ordinaryValues.slot
        }
        pointer.push(1);
    }

    function retargetToProtected() external { //~WARN: protected variable `protectedValues` is written without `onlyOwner()`
        uint256[] storage pointer = ordinaryValues;
        assembly {
            pointer.slot := protectedValues.slot
        }
        pointer.push(1);
    }

    function retargetAwayOnExhaustiveSwitch(uint256 selector) external {
        uint256[] storage pointer = protectedValues;
        assembly {
            switch selector
            case 0 { pointer.slot := ordinaryValues.slot }
            default { pointer.slot := ordinaryValues.slot }
        }
        pointer.push(1);
    }
}

contract TransientStorageWrites {
    /// @custom:security write-protection="onlyOwner()"
    uint256 internal protectedValue;

    function writeTransientStorage() external {
        assembly {
            tstore(protectedValue.slot, 1)
        }
    }

    function writePersistentStorage() external { //~WARN: protected variable `protectedValue` is written without `onlyOwner()`
        assembly {
            sstore(protectedValue.slot, 1)
        }
    }
}

contract YulCallReturnSummaries {
    /// @custom:security write-protection="onlyOwner()"
    uint256 internal protectedValue;

    uint256 internal ordinaryValue;

    function storeAtSecondArgument() external {
        assembly {
            function pickSecond(first, second) -> result {
                result := second
            }
            sstore(pickSecond(protectedValue.slot, ordinaryValue.slot), 1)
        }
    }

    function storeAtFirstArgument() external { //~WARN: protected variable `protectedValue` is written without `onlyOwner()`
        assembly {
            function pickFirst(first, second) -> result {
                result := first
            }
            sstore(pickFirst(protectedValue.slot, ordinaryValue.slot), 1)
        }
    }
}
