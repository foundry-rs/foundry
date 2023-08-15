// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3674
contract Issue3674Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testNonceCreateSelect() public {
        vm.createSelectFork("https://goerli.infura.io/v3/b9794ad1ddf84dfb8c34d6bb5dca2001");

        vm.createSelectFork("https://api.avax-test.network/ext/bc/C/rpc");
        assert(vm.getNonce(msg.sender) > 0x17);
    }
}
