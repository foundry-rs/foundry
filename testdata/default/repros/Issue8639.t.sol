// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";

library ExternalLibrary {
    function doWork(uint256 a) public returns (uint256) {
        return a++;
    }
}

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        ExternalLibrary.doWork(1);
    }

    function increment() public {}
}

// https://github.com/foundry-rs/foundry/issues/8639
contract Issue8639Test is DSTest {
    Counter counter;

    function setUp() public {
        counter = new Counter();
    }

    /// forge-config: default.fuzz.runs = 1000
    /// forge-config: default.fuzz.seed = '100'
    function test_external_library_address(address test) public {
        require(test != address(ExternalLibrary));
    }
}

contract Issue8639AnotherTest is DSTest {
    /// forge-config: default.fuzz.runs = 1000
    /// forge-config: default.fuzz.seed = '100'
    function test_another_external_library_address(address test) public {
        require(test != address(ExternalLibrary));
    }
}
