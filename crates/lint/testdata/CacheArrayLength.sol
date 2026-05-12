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

    function memoryArrayLength(uint256[] memory values) public pure returns (uint256 sum) {
        for (uint256 i = 0; i <= values.length; ++i) { //~NOTE: array length read in loop condition
            if (i < values.length) {
                sum += values[i];
            }
        }
    }

    function calldataArrayLength(uint256[] calldata values) external pure returns (uint256 sum) {
        for (uint256 i = 0; values.length > i; ++i) { //~NOTE: array length read in loop condition
            sum += values[i];
        }
    }

    function bytesLength() external view returns (uint256 sum) {
        for (uint256 i = 0; i < data.length; ++i) { //~NOTE: array length read in loop condition
            sum += uint8(data[i]);
        }
    }

    function nestedArrayLength(uint256[][] memory values) public pure returns (uint256 sum) {
        for (uint256 i = 0; i < values[0].length; ++i) { //~NOTE: array length read in loop condition
            sum += values[0][i];
        }
    }

    function mappingValueArrayLength(uint256 key) external view returns (uint256 sum) {
        for (uint256 i = 0; i < buckets[key].length; ++i) { //~NOTE: array length read in loop condition
            sum += buckets[key][i];
        }
    }

    function structFieldArrayLength() external view returns (uint256 sum) {
        for (uint256 i = 0; i < bag.values.length; ++i) { //~NOTE: array length read in loop condition
            sum += bag.values[i];
        }
    }

    function compoundCondition(uint256[] memory left, uint256[] memory right)
        public
        pure
        returns (uint256 sum)
    {
        for (uint256 i = 0; i < left.length && i < right.length; ++i) { //~NOTE: array length read in loop condition
            //~^NOTE: array length read in loop condition
            sum += left[i] + right[i];
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
}
