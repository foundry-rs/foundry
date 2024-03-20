// SPDX-License-Identifier: MIT
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6006
contract Issue6066Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_parse_11e20_sci() public {
        string memory json = '{"value": 1.1e20}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 1.1e20);
    }

    function test_parse_22e20_sci() public {
        string memory json = '{"value": 2.2e20}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 2.2e20);
    }

    function test_parse_2e_sci() public {
        string memory json = '{"value": 2e10}';
        bytes memory parsed = vm.parseJson(json);
        Value memory data = abi.decode(parsed, (Value));
        assertEq(data.value, 2e10);
    }
}

struct Value {
    uint256 value;
}
