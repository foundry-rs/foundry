//@compile-flags: --only-lint missing-events-access-control

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract MissingEventsAccessControlPaths {
    address public owner = msg.sender;
    mapping(address => bool) public roles;
    mapping(address => bool) private scratch;
    struct RoleData {
        address account;
    }
    RoleData private roleData;

    event OwnershipTransferred(address oldOwner, address newOwner);

    modifier onlyOwner() {
        require(msg.sender == owner);
        _;
    }

    function useRole() external view {
        require(roles[msg.sender]);
    }

    function useRoleData() external view {
        require(roleData.account == msg.sender);
    }

    function optionalEventAfterWrite(address newOwner, bool skip) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        _maybeLogOwner(newOwner, skip);
    }

    function _maybeLogOwner(address newOwner, bool skip) internal {
        if (skip) return;
        emit OwnershipTransferred(owner, newOwner);
    }

    function optionalNestedEvent(address newOwner, bool skip) external onlyOwner {
        _maybeLogOwnerNested(newOwner, skip);
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function _maybeLogOwnerNested(address newOwner, bool skip) internal {
        _maybeLogOwner(newOwner, skip);
    }

    function guaranteedEvent(address newOwner, bool stop) external onlyOwner {
        _alwaysLogOwner(newOwner, stop);
        owner = newOwner;
    }

    function guaranteedEventAfterWrite(address newOwner, bool stop) external onlyOwner {
        owner = newOwner;
        _alwaysLogOwner(newOwner, stop);
    }

    function _alwaysLogOwner(address newOwner, bool stop) internal {
        emit OwnershipTransferred(owner, newOwner);
        if (stop) return;
    }

    function optionalHelperWrite(address newOwner, bool skip) external onlyOwner {
        _setOwnerUnlessSkipped(newOwner, skip);
    }

    function _setOwnerUnlessSkipped(address newOwner, bool skip) internal {
        if (skip) return;
        owner = newOwner;
        emit OwnershipTransferred(owner, newOwner);
    }

    // Distinct branch writes must not be confused when matching return states.
    function branchWrites(address newOwner, bool early) external onlyOwner {
        if (early) {
            owner = newOwner;
            emit OwnershipTransferred(owner, newOwner);
            return;
        }
        owner = newOwner;
        emit OwnershipTransferred(owner, newOwner);
    }

    function uncoveredReturningBranch(address newOwner, bool early) external onlyOwner {
        if (early) {
            owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
            return;
        }
        owner = newOwner;
        emit OwnershipTransferred(owner, newOwner);
    }

    function uncoveredFallthroughBranch(address newOwner, bool early) external onlyOwner {
        if (early) {
            owner = newOwner;
            emit OwnershipTransferred(owner, newOwner);
            return;
        }
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
    }

    function missingEventOnReturn(address newOwner, bool early) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        if (early) return;
        emit OwnershipTransferred(owner, newOwner);
    }

    function missingEventOnSelfdestruct(address newOwner, bool early) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        if (early) {
            emit OwnershipTransferred(owner, newOwner);
            return;
        }
        selfdestruct(payable(msg.sender));
    }

    function eventAfterPossibleSelfdestruct(address newOwner, bool stop) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        if (stop) selfdestruct(payable(msg.sender));
        emit OwnershipTransferred(owner, newOwner);
    }

    function nestedPossibleSelfdestruct(address newOwner, bool stop) external onlyOwner {
        owner = newOwner; //~WARN: `owner` is changed without an event but is used for access control
        _nestedSelfdestruct(stop);
        emit OwnershipTransferred(owner, newOwner);
    }

    function _nestedSelfdestruct(bool stop) internal {
        _maybeSelfdestruct(stop);
    }

    function _maybeSelfdestruct(bool stop) internal {
        if (stop) selfdestruct(payable(msg.sender));
    }

    function writeAfterTerminalOrEvent(address newOwner, bool stop) external onlyOwner {
        _terminalOrLog(newOwner, stop);
        owner = newOwner;
    }

    function _terminalOrLog(address newOwner, bool stop) internal {
        if (stop) selfdestruct(payable(msg.sender));
        emit OwnershipTransferred(owner, newOwner);
    }

    function rebindThenWrite(address account) external onlyOwner {
        mapping(address => bool) storage selected = roles;
        selected = scratch;
        selected[account] = true;
    }

    function rebindToRolesThenWrite(address account) external onlyOwner {
        mapping(address => bool) storage selected = scratch;
        selected = roles;
        selected[account] = true; //~WARN: `roles` is changed without an event but is used for access control
    }

    // Assigning memory data to a member through a storage pointer changes the referenced state.
    function copyMemoryIntoStorage(RoleData memory value) public onlyOwner {
        RoleData storage selected = roleData;
        selected.account = value.account; //~WARN: `roleData` is changed without an event but is used for access control
    }
}
