//@compile-flags: --only-lint return-bomb

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IReturnBombTarget {
    function fetch() external returns (bytes memory);
    function value() external returns (uint256);
}

interface IReturnBombStructTarget {
    struct Result {
        bytes data;
    }

    function fetch() external returns (Result memory);
}

interface IReturnBombFalsePositiveTarget {
    function call(bytes calldata payload) external returns (uint256);
}

interface IReturnBombOverloadedTarget {
    function fetch(uint256 value) external returns (uint256);
    function fetch(bytes calldata payload) external returns (bytes memory);
}

interface IReturnBombOverloadedReverseTarget {
    function fetch(bytes calldata payload) external returns (bytes memory);
    function fetch(uint256 value) external returns (uint256);
}

interface IReturnBombAmbiguousOverloadedTarget {
    function fetch(uint256 value) external returns (bytes memory);
    function fetch(bool value) external returns (uint256);
}

interface IReturnBombWideningTarget {
    function fetch(uint256 value) external returns (bytes memory);
}

interface IReturnBombLiteralRangeTarget {
    function fetch(uint8 value) external returns (uint256);
    function fetch(uint256 value) external returns (bytes memory);
}

interface IReturnBombNegativeLiteralTarget {
    function fetch(int8 value) external returns (uint256);
    function fetch(uint256 value) external returns (bytes memory);
}

contract ReturnBombBaseArgument {}

contract ReturnBombDerivedArgument is ReturnBombBaseArgument {}

interface IReturnBombBaseArgumentTarget {
    function fetch(ReturnBombBaseArgument value) external returns (bytes memory);
}

interface IReturnBombFixedArrayOverloadedTarget {
    function fetch(uint256[2] calldata values) external returns (bytes memory);
    function fetch(uint256[3] calldata values) external returns (uint256);
}

uint256 constant RETURN_BOMB_FIXED_ARRAY_BASE = 1;
uint256 constant RETURN_BOMB_FIXED_ARRAY_LENGTH = RETURN_BOMB_FIXED_ARRAY_BASE + 1;

interface IReturnBombConstantFixedArrayTarget {
    function fetch(uint256[RETURN_BOMB_FIXED_ARRAY_LENGTH] calldata values) external returns (bytes memory);
}

contract ReturnBombCreatedTarget {
    function fetch() external returns (bytes memory) {
        return "";
    }
}

contract ReturnBomb {
    event DynamicReturn(bytes result);

    error DynamicReturnError(bytes result);

    bytes initializedData = this.ownDynamicReturn{gas: 10_000}(); //~WARN: external calls with a gas limit should not consume unbounded return data

    struct Result {
        bytes data;
    }

    struct TargetSlot {
        IReturnBombTarget target;
    }

    struct FunctionPointerSlot {
        function() external returns (bytes memory) fetcher;
    }

    bytes existingData;
    bytes[] results;
    Result storedResult;
    mapping(uint256 => IReturnBombTarget) mappedTargets;
    IReturnBombTarget storedTarget;

    // SHOULD PASS: Calls without a gas cap.
    function valueOnlyOption(address payable target, bytes memory payload, uint256 value) public {
        (bool success, bytes memory result) = target.call{value: value}(payload);
        require(success, "Call failed");
        require(result.length >= 0);
    }

    // SHOULD FAIL: Gas-capped low-level calls that copy unbounded returndata.
    function ignoresReturnData(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, ) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
    }

    function standaloneLowLevelCall(address target, bytes memory payload, uint256 gasLimit) public {
        target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function lowLevelCall(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, bytes memory result) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
        require(result.length >= 0);
    }

    function lowLevelCallLiteralGas(address target, bytes memory payload) public {
        (bool success, bytes memory result) = target.call{gas: 10_000}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
        require(result.length >= 0);
    }

    function lowLevelCallWithValue(address payable target, bytes memory payload, uint256 gasLimit, uint256 value) public {
        (bool success, bytes memory result) = target.call{value: value, gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
        require(result.length >= 0);
    }

    function delegateCall(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, bytes memory result) = target.delegatecall{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Delegatecall failed");
        require(result.length >= 0);
    }

    function staticCall(address target, bytes memory payload, uint256 gasLimit) public view {
        (bool success, bytes memory result) = target.staticcall{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Staticcall failed");
        require(result.length >= 0);
    }

    function msgSenderCall(bytes memory payload, uint256 gasLimit) public {
        msg.sender.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function blockCoinbaseCall(bytes memory payload, uint256 gasLimit) public {
        block.coinbase.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function txOriginCall(bytes memory payload, uint256 gasLimit) public {
        tx.origin.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function getAddress() public view returns (address) {
        return address(this);
    }

    function returnedAddressLowLevelCall(bytes memory payload, uint256 gasLimit) public {
        getAddress().call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function ecrecoverLowLevelCall(
        bytes32 h,
        uint8 v,
        bytes32 r,
        bytes32 s,
        bytes memory payload,
        uint256 gasLimit
    ) public {
        ecrecover(h, v, r, s).call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function existingBytes(address target, bytes memory payload, uint256 gasLimit) public {
        (bool success, bytes memory result) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        (success, existingData) = target.call{gas: gasLimit}(result); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
    }

    function directReturn(address target, bytes memory payload, uint256 gasLimit) public returns (bool, bytes memory) {
        return target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function memberBytes(address target, bytes memory payload, uint256 gasLimit) public {
        bool success;
        (success, storedResult.data) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
    }

    function indexedBytes(address target, bytes memory payload, uint256 gasLimit) public {
        bool success;
        results.push();
        (success, results[0]) = target.call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(success, "Call failed");
    }

    function highLevelDynamicReturn(IReturnBombTarget target, uint256 gasLimit) public {
        bytes memory result = target.fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function standaloneHighLevelDynamicReturn(IReturnBombTarget target, uint256 gasLimit) public {
        target.fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function nestedHighLevelDynamicReturn(IReturnBombTarget target, uint256 gasLimit) public {
        require(target.fetch{gas: gasLimit}().length >= 0); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function directHighLevelDynamicReturn(IReturnBombTarget target, uint256 gasLimit) public returns (bytes memory) {
        return target.fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function highLevelDynamicStructReturn(IReturnBombStructTarget target, uint256 gasLimit) public {
        IReturnBombStructTarget.Result memory result = target.fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.data.length >= 0);
    }

    function indexedHighLevelDynamicReturn(IReturnBombTarget[] memory targets, uint256 index, uint256 gasLimit) public {
        bytes memory result = targets[index].fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function mappedHighLevelDynamicReturn(uint256 index, uint256 gasLimit) public {
        bytes memory result = mappedTargets[index].fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function memberHighLevelDynamicReturn(TargetSlot memory slot, uint256 gasLimit) public {
        bytes memory result = slot.target.fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function getTarget() public view returns (IReturnBombTarget) {
        return storedTarget;
    }

    function returnedTargetHighLevelDynamicReturn(uint256 gasLimit) public {
        bytes memory result = getTarget().fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function ownDynamicReturn() external returns (bytes memory) {
        return "";
    }

    function externalStaticOverload(uint256 value) external returns (uint256) {
        return value;
    }

    function externalStaticOverload(bool) internal returns (bytes memory) {
        return "";
    }

    function thisHighLevelDynamicReturn(uint256 gasLimit) public {
        this.ownDynamicReturn{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    modifier withDynamicReturn(bytes memory) {
        _;
    }

    function thisHighLevelDynamicReturnInModifierArgument(uint256 gasLimit)
        public
        withDynamicReturn(this.ownDynamicReturn{gas: gasLimit}()) //~WARN: external calls with a gas limit should not consume unbounded return data
    {}

    function thisExternalStaticOverloadIgnoresInternal(uint256 value, uint256 gasLimit) public {
        uint256 result = this.externalStaticOverload{gas: gasLimit}(value + 1);
        require(result >= 0);
    }

    function highLevelStaticReturn(IReturnBombTarget target, uint256 gasLimit) public {
        uint256 result = target.value{gas: gasLimit}();
        require(result >= 0);
    }

    function highLevelDynamicReturnInIf(IReturnBombTarget target, uint256 gasLimit) public {
        if (target.fetch{gas: gasLimit}().length > 0) {} //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function highLevelDynamicReturnInEmit(IReturnBombTarget target, uint256 gasLimit) public {
        emit DynamicReturn(target.fetch{gas: gasLimit}()); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function highLevelDynamicReturnInRevert(IReturnBombTarget target, uint256 gasLimit) public {
        revert DynamicReturnError(target.fetch{gas: gasLimit}()); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function highLevelDynamicReturnInTry(IReturnBombTarget target, uint256 gasLimit) public {
        try target.fetch{gas: gasLimit}() returns (bytes memory) { //~WARN: external calls with a gas limit should not consume unbounded return data
        } catch {}
    }

    function highLevelMethodNamedCall(IReturnBombFalsePositiveTarget target, bytes calldata payload, uint256 gasLimit) public {
        uint256 result = target.call{gas: gasLimit}(payload);
        require(result >= 0);
    }

    function overloadedDynamicReturn(
        IReturnBombOverloadedTarget target,
        bytes calldata payload,
        uint256 gasLimit
    ) public {
        bytes memory result = target.fetch{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function overloadedStaticReturn(
        IReturnBombOverloadedReverseTarget target,
        uint256 value,
        uint256 gasLimit
    ) public {
        uint256 result = target.fetch{gas: gasLimit}(value);
        require(result >= 0);
    }

    function overloadedUnknownArgumentDynamicReturn(
        IReturnBombAmbiguousOverloadedTarget target,
        uint256 value,
        uint256 gasLimit
    ) public {
        bytes memory result = target.fetch{gas: gasLimit}(value + 1); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function integerWideningDynamicReturn(
        IReturnBombWideningTarget target,
        uint8 value,
        uint256 gasLimit
    ) public {
        bytes memory result = target.fetch{gas: gasLimit}(value); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function overloadedBooleanExpressionStaticReturn(
        IReturnBombAmbiguousOverloadedTarget target,
        uint256 value,
        uint256 gasLimit
    ) public {
        uint256 result = target.fetch{gas: gasLimit}(value > 0);
        require(result >= 0);
    }

    function overloadedLiteralRangeDynamicReturn(
        IReturnBombLiteralRangeTarget target,
        uint256 gasLimit
    ) public {
        bytes memory result = target.fetch{gas: gasLimit}(300); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function overloadedNegativeLiteralStaticReturn(
        IReturnBombNegativeLiteralTarget target,
        uint256 gasLimit
    ) public {
        uint256 result = target.fetch{gas: gasLimit}(-1);
        require(result >= 0);
    }

    function derivedContractArgumentDynamicReturn(
        IReturnBombBaseArgumentTarget target,
        ReturnBombDerivedArgument value,
        uint256 gasLimit
    ) public {
        bytes memory result = target.fetch{gas: gasLimit}(value); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function fixedArrayOverloadDynamicReturn(
        IReturnBombFixedArrayOverloadedTarget target,
        uint256[2] calldata values,
        uint256 gasLimit
    ) public {
        bytes memory result = target.fetch{gas: gasLimit}(values); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function constantFixedArrayLengthDynamicReturn(
        IReturnBombConstantFixedArrayTarget target,
        uint256[RETURN_BOMB_FIXED_ARRAY_BASE + 1] calldata values,
        uint256 gasLimit
    ) public {
        bytes memory result = target.fetch{gas: gasLimit}(values); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function ternaryHighLevelDynamicReturn(
        IReturnBombTarget first,
        IReturnBombTarget second,
        bool useFirst,
        uint256 gasLimit
    ) public {
        bytes memory result = (useFirst ? first : second).fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function ternaryLowLevelCall(
        address first,
        address second,
        bool useFirst,
        bytes memory payload,
        uint256 gasLimit
    ) public {
        (useFirst ? first : second).call{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
    }

    function castDynamicReturn(address target, uint256 gasLimit) public {
        bytes memory result = IReturnBombTarget(target).fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function castOverloadedDynamicReturn(
        address target,
        bytes calldata payload,
        uint256 gasLimit
    ) public {
        bytes memory result = IReturnBombOverloadedTarget(target).fetch{gas: gasLimit}(payload); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function externalFunctionPointerDynamicReturn(
        function() external returns (bytes memory) fetcher,
        uint256 gasLimit
    ) public {
        bytes memory result = fetcher{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function externalFunctionPointerMemberDynamicReturn(
        FunctionPointerSlot memory slot,
        uint256 gasLimit
    ) public {
        bytes memory result = slot.fetcher{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function getFetcher() public view returns (function() external returns (bytes memory)) {
        return this.ownDynamicReturn;
    }

    function returnedExternalFunctionPointerDynamicReturn(uint256 gasLimit) public {
        bytes memory result = getFetcher(){gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function externalFunctionPointerReturnedTargetDynamicReturn(
        function() external returns (IReturnBombTarget) targetGetter,
        uint256 gasLimit
    ) public {
        bytes memory result = targetGetter().fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }

    function newTargetDynamicReturn(uint256 gasLimit) public {
        bytes memory result = (new ReturnBombCreatedTarget()).fetch{gas: gasLimit}(); //~WARN: external calls with a gas limit should not consume unbounded return data
        require(result.length >= 0);
    }
}
