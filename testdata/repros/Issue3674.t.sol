// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3674
contract Issue3674Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function testNonceCreateSelect() public {
        vm.createSelectFork("https://goerli.infura.io/v3/f4a0bdad42674adab5fc0ac077ffab2b");

        vm.createSelectFork("https://api.avax-test.network/ext/bc/C/rpc");
        assert(vm.getNonce(msg.sender) > 0x17);
    }
}
