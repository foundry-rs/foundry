// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for the `mapping-deletion` lint, which flags `delete` on a value whose type contains a
// mapping. `delete` zeroes the non-mapping members but cannot clear the mapping's entries, leaving
// stale storage behind.

contract MappingDeletion {
    struct WithMapping {
        uint256 total;
        mapping(address => uint256) balances;
    }

    struct Nested {
        uint256 x;
        WithMapping inner;
    }

    struct Plain {
        uint256 a;
        uint256 b;
    }

    WithMapping internal s;
    Nested internal n;
    Plain internal p;
    WithMapping[] internal arr;
    mapping(uint256 => WithMapping) internal m;
    mapping(uint256 => uint256) internal plainMap;

    // SHOULD FAIL: deleting a struct that directly holds a mapping.
    function badStruct() external {
        delete s; //~WARN: `delete` on a value containing a mapping does not clear the mapping
    }

    // SHOULD FAIL: the mapping is reachable through a nested struct.
    function badNested() external {
        delete n; //~WARN: `delete` on a value containing a mapping does not clear the mapping
    }

    // SHOULD FAIL: deleting an array of structs that hold a mapping.
    function badArray() external {
        delete arr; //~WARN: `delete` on a value containing a mapping does not clear the mapping
    }

    // SHOULD FAIL: the mapping value is itself a struct with a mapping.
    function badMappingValue(uint256 id) external {
        delete m[id]; //~WARN: `delete` on a value containing a mapping does not clear the mapping
    }

    // SHOULD FAIL: a single array element is a struct with a mapping.
    function badArrayElem(uint256 i) external {
        delete arr[i]; //~WARN: `delete` on a value containing a mapping does not clear the mapping
    }

    // OK: plain struct, no mapping reachable.
    function okPlain() external {
        delete p;
    }

    // OK: deleting a scalar member.
    function okScalarField() external {
        delete s.total;
    }

    // OK: deleting a single entry of a plain mapping.
    function okMappingEntry(uint256 k) external {
        delete plainMap[k];
    }
}
