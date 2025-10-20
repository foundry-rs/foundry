// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.1;

import "utils/Test.sol";

contract Backdoor {
    uint256 public number = 1;

    function backdoor(uint256 newNumber) public payable {
        uint256 x = newNumber - 1;
        if (x == 6912213124124531) {
            number = 0;
        }
    }
}

// https://github.com/foundry-rs/foundry/issues/2851
contract Issue2851Test is Test {
    Backdoor back;

    function setUp() public {
        back = new Backdoor();
    }

    /// forge-config: default.fuzz.dictionary.max_fuzz_dictionary_literals = 0
    /// forge-config: default.fuzz.seed = '111'
    function invariantNotZero() public {
        assertEq(back.number(), 1);
    }
}
