//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract NamedStructFields {
    struct Person {
        string name;
        uint256 age;
        address wallet;
    }

    function namedArgs() public {
        Person memory person = Person({
            name: "Alice",
            age: 25,
            wallet: address(0)
        });
    }

    function positionalArgs() public {
        Person memory person = Person("Alice", 25, address(0)); //~NOTE: prefer initializing structs with named fields
    }
}
