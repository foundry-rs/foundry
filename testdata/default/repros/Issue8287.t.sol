// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/8287
contract Issue8287Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRpcBalance() public {
        uint256 f2 = vm.createSelectFork("mainnet", 10);
        bytes memory data = vm.rpc("eth_getBalance", "[\"0x551e7784778ef8e048e495df49f2614f84a4f1dc\",\"0x0\"]");
        string memory m = vm.toString(data);
        assertEq(m, "0x2086ac351052600000");
    }

    function testRpcStorage() public {
        uint256 f2 = vm.createSelectFork("mainnet", 10);
        bytes memory data = vm.rpc(
            "eth_getStorageAt",
            "[\"0x551e7784778ef8e048e495df49f2614f84a4f1dc\",\"0x40BdB4497614bAe1A67061EE20AAdE3c2067AC9e\",\"0x0\"]"
        );
        string memory m = vm.toString(data);
        assertEq(m, "0x0000000000000000000000000000000000000000000000000000000000000000");
    }
}
