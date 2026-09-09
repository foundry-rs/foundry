//@compile-flags: --only-lint cache-array-length

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract CacheArrayLength {
    uint256[] internal items;
    bytes internal data;
    mapping(uint256 => uint256[]) internal buckets;
    uint256[3] internal fixedItems;

    struct Bag {
        uint256[] values;
    }

    struct Counter {
        uint256 length;
    }

    Bag internal bag;
    Counter internal counter;

    function storageArrayLength() external view returns (uint256 sum) {
        for (uint256 i = 0; i < items.length; ++i) { //~NOTE: array length read in loop condition
            sum += items[i];
        }
    }

    function compoundConditionStorageArrayLength(uint256 cap)
        external
        view
        returns (uint256 sum)
    {
        for (uint256 i = 0; i < items.length && i < cap; ++i) { //~NOTE: array length read in loop condition
            sum += items[i];
        }
    }

    function reversedCompoundConditionStorageArrayLength(uint256 cap)
        external
        view
        returns (uint256 sum)
    {
        for (uint256 i = 0; i < cap && items.length > i; ++i) { //~NOTE: array length read in loop condition
            sum += items[i];
        }
    }

    function memoryArrayLength(uint256[] memory values) public pure returns (uint256 sum) {
        for (uint256 i = 0; i <= values.length; ++i) {
            if (i < values.length) {
                sum += values[i];
            }
        }
    }

    function calldataArrayLength(uint256[] calldata values) external pure returns (uint256 sum) {
        for (uint256 i = 0; values.length > i; ++i) {
            sum += values[i];
        }
    }

    function bytesLength() external view returns (uint256 sum) {
        for (uint256 i = 0; i < data.length; ++i) {
            sum += uint8(data[i]);
        }
    }

    function bytesCastLength(string memory value) public pure returns (uint256 sum) {
        for (uint256 i = 0; i < bytes(value).length; ++i) {
            sum += uint8(bytes(value)[i]);
        }
    }

    function nestedArrayLength(uint256[][] memory values) public pure returns (uint256 sum) {
        for (uint256 i = 0; i < values[0].length; ++i) {
            sum += values[0][i];
        }
    }

    function mappingValueArrayLength(uint256 key) external view returns (uint256 sum) {
        for (uint256 i = 0; i < buckets[key].length; ++i) {
            sum += buckets[key][i];
        }
    }

    function structFieldArrayLength() external view returns (uint256 sum) {
        for (uint256 i = 0; i < bag.values.length; ++i) {
            sum += bag.values[i];
        }
    }

    function compoundCondition(uint256[] memory left, uint256[] memory right)
        public
        pure
        returns (uint256 sum)
    {
        for (uint256 i = 0; i < left.length && i < right.length; ++i) {
            sum += left[i] + right[i];
        }
    }

    function functionReturnArrayLength() public pure returns (uint256 sum) {
        for (uint256 i = 0; i < generatedValues().length; ++i) {
            sum += i;
        }
    }

    function cachedLength(uint256[] memory values) public pure returns (uint256 sum) {
        uint256 length = values.length;
        for (uint256 i = 0; i < length; ++i) {
            sum += values[i];
        }
    }

    function fixedArrayLength() external view returns (uint256 sum) {
        for (uint256 i = 0; i < fixedItems.length; ++i) {
            sum += fixedItems[i];
        }
    }

    function nonArrayLengthField() external view returns (uint256 sum) {
        for (uint256 i = 0; i < counter.length; ++i) {
            sum += i;
        }
    }

    function storageArrayPushLengthMutation() external {
        for (uint256 i = 0; i < items.length; ++i) {
            items.push(i);
        }
    }

    function storageArrayPopLengthMutation() external {
        for (uint256 i = 0; i < items.length; ++i) {
            items.pop();
        }
    }

    function storageArrayAliasLengthMutation() external {
        uint256[] storage aliasedItems = items;
        for (uint256 i = 0; i < items.length; ++i) {
            aliasedItems.push(i);
        }
    }

    function storageArrayPostExpressionMutation() external {
        for (uint256 i = 0; i < items.length; items.pop()) {}
    }

    function storageArrayHelperMutation() external {
        for (uint256 i = 0; i < items.length; ++i) {
            mutateItems(i);
        }
    }

    function conditionHelperMutation(uint256 cap) external returns (uint256 sum) {
        for (uint256 i = 0; i < items.length && mutateAndContinue(i, cap); ++i) {
            sum += i;
        }
    }

    function loopVariantNestedArrayLength(uint256[][] memory values)
        public
        pure
        returns (uint256 sum)
    {
        for (uint256 i = 0; i < values[i].length; ++i) {
            sum += i;
        }
    }

    function sideEffectingReturnArrayLength() external returns (uint256 sum) {
        for (uint256 i = 0; i < mutatingValues().length; ++i) {
            sum += i;
        }
    }

    function pureReturnArrayLengthWithLoopVariantArg(uint256[][] memory values)
        public
        pure
        returns (uint256 sum)
    {
        for (uint256 i = 0; i < pickValues(values, i).length; ++i) {
            sum += i;
        }
    }

    function generatedValues() internal pure returns (uint256[] memory values) {
        values = new uint256[](3);
    }

    function mutateItems(uint256 value) internal {
        items.push(value);
    }

    function mutateAndContinue(uint256 value, uint256 cap) internal returns (bool) {
        items.push(value);
        return value < cap;
    }

    function mutatingValues() internal returns (uint256[] memory values) {
        items.push(1);
        values = new uint256[](3);
    }

    function pickValues(uint256[][] memory values, uint256 index)
        internal
        pure
        returns (uint256[] memory)
    {
        return values[index];
    }
}
