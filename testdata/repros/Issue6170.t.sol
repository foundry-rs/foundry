// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

contract Emitter {
    event Values(uint256 indexed a, uint256 indexed b);

    function plsEmit(uint256 a, uint256 b) external {
        emit Values(a, b);
    }
}

// https://github.com/foundry-rs/foundry/issues/6170
contract Issue6170Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    event Values(uint256 indexed a, uint256 b);

    Emitter e = new Emitter();

    function test() public {
        vm.expectEmit(true, true, false, true);
        emit Values(69, 420);
        e.plsEmit(69, 420);
    }
}
