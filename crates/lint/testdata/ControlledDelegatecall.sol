//@compile-flags: --only-lint controlled-delegatecall

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface OrdinaryDelegatecall {
    function delegatecall(bytes calldata data) external returns (bool);
}

interface OverloadedDelegatecallFactory {
    function target(uint256 id) external returns (address);
    function target(bytes calldata data) external returns (OrdinaryDelegatecall);
}

contract ControlledDelegatecall {
    struct OrdinaryDelegatecallHolder {
        OrdinaryDelegatecall target;
    }

    struct AddressHolder {
        address target;
        address sibling;
    }

    address public implementation;
    address public immutable trustedImplementation;
    address public constant TRUSTED = 0x000000000000000000000000000000000000dEaD;
    mapping(address => address) public plugins;
    AddressHolder private storedHolder;
    OrdinaryDelegatecallHolder ordinaryHolder;
    OrdinaryDelegatecall[] ordinaryTargets;
    OverloadedDelegatecallFactory overloadedFactory;

    constructor(address _trusted) {
        trustedImplementation = _trusted;
    }

    function delegateToParameter(address target, bytes calldata data) external returns (bool ok) {
        (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToPayableParameter(
        address payable target,
        bytes calldata data
    ) external returns (bool ok) {
        (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToParameterCast(address target, bytes calldata data) external returns (bool ok) {
        (ok,) = address(target).delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToAssignedParameter(address target, bytes calldata data) external returns (bool ok) {
        address localTarget = target;
        (ok,) = localTarget.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToTernaryParameter(
        address target,
        bool useTarget,
        bytes calldata data
    ) external returns (bool ok) {
        (ok,) = (useTarget ? target : TRUSTED).delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToStorage(bytes calldata data) external returns (bool ok) {
        (ok,) = implementation.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToMapping(address user, bytes calldata data) external returns (bool ok) {
        (ok,) = plugins[user].delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToMsgSender(bytes calldata data) external returns (bool ok) {
        (ok,) = msg.sender.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function protectedDelegateToParameter(
        address target,
        bytes calldata data
    ) external onlyOwner returns (bool ok) {
        (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToImmutable(bytes calldata data) external returns (bool ok) {
        (ok,) = trustedImplementation.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToConstant(bytes calldata data) external returns (bool ok) {
        (ok,) = TRUSTED.delegatecall(data);
    }

    function delegateToLiteral(bytes calldata data) external returns (bool ok) {
        (ok,) = address(0x000000000000000000000000000000000000dEaD).delegatecall(data);
    }

    function delegateToSelf(bytes calldata data) external returns (bool ok) {
        (ok,) = address(this).delegatecall(data);
    }

    function ordinaryDelegatecallFunction(OrdinaryDelegatecall target, bytes calldata data) external returns (bool ok) {
        ok = target.delegatecall(data);
    }

    function ordinaryDelegatecallFromStruct(bytes calldata data) external returns (bool ok) {
        ok = ordinaryHolder.target.delegatecall(data);
    }

    function ordinaryDelegatecallFromArray(uint256 index, bytes calldata data) external returns (bool ok) {
        ok = ordinaryTargets[index].delegatecall(data);
    }

    function ordinaryDelegatecallFromOverload(bytes calldata data) external returns (bool ok) {
        ok = overloadedFactory.target(data).delegatecall(data);
    }

    function delegateToGuarded(address target, bytes calldata data) external returns (bool ok) {
        require(target == trustedImplementation);
        (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToConstantGuarded(address target, bytes calldata data) external returns (bool ok) {
        require(target == TRUSTED);
        (ok,) = target.delegatecall(data);
    }

    function delegateToConstantGuardedByRevert(address target, bytes calldata data) external returns (bool ok) {
        if (target != TRUSTED) revert();
        (ok,) = target.delegatecall(data);
    }

    function delegateToModifierGuarded(address target, bytes calldata data) external onlyTrusted(target) returns (bool ok) {
        (ok,) = target.delegatecall(data);
    }

    function id(address target) internal pure returns (address) {
        return target;
    }

    function impl() internal view returns (address) {
        return implementation;
    }

    function trusted() internal pure returns (address) {
        return TRUSTED;
    }

    function nestedTrusted() internal pure returns (address) {
        return trusted();
    }

    function branchedTrusted(bool branch) internal pure returns (address) {
        if (branch) return nestedTrusted();
        return TRUSTED;
    }

    function recursiveTrusted(uint256 depth) internal pure returns (address) {
        if (depth == 0) return TRUSTED;
        return recursiveTrusted(depth - 1);
    }

    function delegateToHelperReturn(address target, bytes calldata data) external returns (bool ok) {
        (ok,) = id(target).delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToStorageHelperReturn(bytes calldata data) external returns (bool ok) {
        (ok,) = impl().delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToTrustedHelperReturn(bytes calldata data) external returns (bool ok) {
        (ok,) = trusted().delegatecall(data);
    }

    function delegateToNestedTrustedHelper(bytes calldata data) external returns (bool ok) {
        (ok,) = nestedTrusted().delegatecall(data);
    }

    function delegateToBranchedTrustedHelper(bool branch, bytes calldata data) external returns (bool ok) {
        (ok,) = branchedTrusted(branch).delegatecall(data);
    }

    function delegateToRecursiveTrustedHelper(uint256 depth, bytes calldata data) external returns (bool ok) {
        (ok,) = recursiveTrusted(depth).delegatecall(data);
    }

    function delegateToDecoded(bytes calldata blob, bytes calldata data) external returns (bool ok) {
        (ok,) = abi.decode(blob, (address)).delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToAssignmentReceiver(address target, bytes calldata data) external returns (bool ok) {
        address localTarget;
        (ok,) = (localTarget = target).delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToTupleReassigned(address target, bytes calldata data) external returns (bool ok) {
        address localTarget = TRUSTED;
        (localTarget,) = (target, uint256(0));
        (ok,) = localTarget.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToTupleHelper(address target, bytes calldata data) external returns (bool ok) {
        address localTarget = TRUSTED;
        (localTarget,) = pair(target);
        (ok,) = localTarget.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function pair(address target) internal pure returns (address, uint256) {
        return (target, 0);
    }

    function delegateToBranchJoin(bool useTarget, address target, bytes calldata data) external returns (bool ok) {
        address localTarget;
        if (useTarget) {
            localTarget = target;
        } else {
            localTarget = TRUSTED;
        }
        (ok,) = localTarget.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToImplicitElseJoin(bool useTrusted, address target, bytes calldata data) external returns (bool ok) {
        address localTarget = target;
        if (useTrusted) {
            localTarget = TRUSTED;
        }
        (ok,) = localTarget.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToLoopJoin(bool useTrusted, address target, bytes calldata data) external returns (bool ok) {
        address localTarget = target;
        while (useTrusted) {
            localTarget = TRUSTED;
        }
        (ok,) = localTarget.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToExitingBranch(bool useTarget, address target, bytes calldata data) external returns (bool ok) {
        address localTarget = TRUSTED;
        if (useTarget) {
            localTarget = target;
            revert();
        }
        (ok,) = localTarget.delegatecall(data);
    }

    function delegateAfterReturn(address target, bytes calldata data) external returns (bool ok) {
        return false;
        (ok,) = target.delegatecall(data);
    }

    function delegateToShortCircuit(bool skipAssignment, address target, bytes calldata data) external returns (bool ok) {
        address localTarget = target;
        if (skipAssignment || (localTarget = TRUSTED) == TRUSTED) {}
        (ok,) = localTarget.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToTernarySideEffect(bool skipAssignment, address target, bytes calldata data) external returns (bool ok) {
        address localTarget = target;
        skipAssignment ? localTarget : (localTarget = TRUSTED);
        (ok,) = localTarget.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToZero(bytes calldata data) external returns (bool ok) {
        (ok,) = address(0).delegatecall(data);
    }

    function delegateToNumericCast(bytes calldata data) external returns (bool ok) {
        (ok,) = address(uint160(0x000000000000000000000000000000000000dEaD)).delegatecall(data);
    }

    function delegateToDeleted(address target, bytes calldata data) external returns (bool ok) {
        address localTarget = target;
        delete localTarget;
        (ok,) = localTarget.delegatecall(data);
    }

    function delegateAfterSideEffectingRequire(
        address target,
        bytes calldata data
    ) external returns (bool ok) {
        require(target == TRUSTED && (target = msg.sender) != address(0));
        (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateAfterSideEffectingRequireMessage(
        address target,
        bytes calldata data
    ) external returns (bool ok) {
        require(target == TRUSTED, string(abi.encodePacked(target = msg.sender)));
        (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateInSideEffectingIf(
        address target,
        bytes calldata data
    ) external returns (bool ok) {
        if (target == TRUSTED && (target = msg.sender) != address(0)) {
            (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
        }
    }

    function delegateToGetter(bytes calldata data) external returns (bool ok) {
        (ok,) = this.implementation().delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToFunctionPointer(
        function () external returns (address) getTarget,
        bytes calldata data
    ) external returns (bool ok) {
        (ok,) = getTarget().delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateToArrayLiteral(address target, bytes calldata data) external returns (bool ok) {
        (ok,) = [target][0].delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateAfterDoWhile(bytes calldata data) external returns (bool ok) {
        address target;
        do {
            target = TRUSTED;
        } while (false);
        (ok,) = target.delegatecall(data);
    }

    function delegateAfterDoWhileConditionSideEffect(bytes calldata data) external returns (bool ok) {
        address target;
        do {
            target = TRUSTED;
        } while ((target = msg.sender) != address(0));
        (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateAfterLossyGuard(address target, bytes calldata data) external returns (bool ok) {
        require(uint8(uint160(target)) == uint8(uint160(TRUSTED)));
        (ok,) = target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    modifier onlyOwner() {
        require(msg.sender == address(0x000000000000000000000000000000000000bEEF));
        _;
    }

    modifier onlyTrusted(address target) {
        require(target == TRUSTED);
        _;
    }

    function setImplementation(address newImplementation) external {
        implementation = newImplementation;
    }

    function setPlugin(address user, address plugin) external {
        plugins[user] = plugin;
    }

    function delegateToTrustedStruct(bytes calldata data) external returns (bool ok) {
        AddressHolder memory holder;
        holder.target = TRUSTED;
        (ok,) = holder.target.delegatecall(data);
    }

    function delegateToUntrustedStruct(address target, bytes calldata data) external returns (bool ok) {
        AddressHolder memory holder;
        holder.target = target;
        (ok,) = holder.target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateAfterSiblingGuard(AddressHolder memory holder, bytes calldata data)
        external
        returns (bool ok)
    {
        require(holder.sibling == TRUSTED);
        (ok,) = holder.target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateAfterProjectedAliasGuard(AddressHolder memory holder, bytes calldata data)
        external
        returns (bool ok)
    {
        address aliasTarget = holder.target;
        require(aliasTarget == TRUSTED);
        (ok,) = holder.target.delegatecall(data);
    }

    function delegateAfterDynamicIndexGuard(
        address[] memory targets,
        uint256 checked,
        uint256 used,
        bytes calldata data
    ) external returns (bool ok) {
        require(targets[checked] == TRUSTED);
        (ok,) = targets[used].delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateAfterMemoryAliasWrite(
        AddressHolder memory holder,
        address attacker,
        bytes calldata data
    ) external returns (bool ok) {
        AddressHolder memory alias_ = holder;
        require(holder.target == TRUSTED);
        alias_.target = attacker;
        (ok,) = holder.target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function delegateAfterConditionalAliasWrite(
        AddressHolder memory first,
        AddressHolder memory second,
        bool chooseFirst,
        address attacker,
        bytes calldata data
    ) external returns (bool ok) {
        require(first.target == TRUSTED);
        require(second.target == TRUSTED);
        AddressHolder memory selected = chooseFirst ? first : second;
        selected.target = attacker;
        (ok,) = first.target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }

    function makeAddressHolder(address target) internal pure returns (AddressHolder memory holder) {
        holder.target = target;
    }

    function delegateAfterDetachedAliasGuard(
        AddressHolder memory holder,
        address attacker,
        bytes calldata data
    ) external returns (bool ok) {
        address target = holder.target;
        AddressHolder memory alias_ = holder;
        holder = makeAddressHolder(attacker);
        require(alias_.target == TRUSTED);
        (ok,) = target.delegatecall(data);
    }

    function delegateAfterRepeatedFreshAggregate(address attacker, bytes calldata data)
        external
        returns (bool ok)
    {
        AddressHolder memory saved;
        saved.target = TRUSTED;
        for (uint256 i; i < 2; ++i) {
            AddressHolder memory current = makeAddressHolder(attacker);
            if (i == 0) {
                saved = current;
            } else {
                current.target = TRUSTED;
                (ok,) = saved.target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
            }
        }
    }

    function delegateAfterStorageAliasWrite(address attacker, bytes calldata data)
        external
        returns (bool ok)
    {
        AddressHolder storage first = storedHolder;
        AddressHolder storage second = storedHolder;
        require(first.target == TRUSTED);
        second.target = attacker;
        (ok,) = first.target.delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }
}

contract ControlledDelegatecallFactory {
    function impl() external view returns (address) {
        return msg.sender;
    }
}

contract ControlledDelegatecallNew {
    function delegateToNew(bytes calldata data) external returns (bool ok) {
        (ok,) = new ControlledDelegatecallFactory().impl().delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }
}

contract ControlledDelegatecallBase {
    address public constant TRUSTED = 0x000000000000000000000000000000000000dEaD;

    function impl() public view virtual returns (address) {
        return TRUSTED;
    }

    function delegateToVirtualHelper(bytes calldata data) external returns (bool ok) {
        (ok,) = impl().delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }
}

contract ControlledDelegatecallBad is ControlledDelegatecallBase {
    function impl() public view override returns (address) {
        return msg.sender;
    }
}

contract ControlledDelegatecallSuperBase {
    function impl() public view virtual returns (address) {
        return msg.sender;
    }
}

contract ControlledDelegatecallSuperChild is ControlledDelegatecallSuperBase {
    function delegateToSuper(bytes calldata data) external returns (bool ok) {
        (ok,) = super.impl().delegatecall(data); //~WARN: delegatecall target is not provably trusted
    }
}
