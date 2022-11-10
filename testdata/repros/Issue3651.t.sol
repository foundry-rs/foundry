// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/3651
contract Issue3651Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function setUp() public {}

    function testDeal() public {
        uint256 fromPrivateKey = 0x1234;
        address from = vm.addr(fromPrivateKey);
        // arbitrary blocknum
        vm.createSelectFork("rpcAlias", 12880747);
        vm.startPrank(from);
        vm.deal(from, 1 ether);

        // random address
        address recipient = 0x3448914BF1fC28c0c8303c422f66C9d438e3D5d5;
        address(recipient).call{value: 0.1 ether}("");
        assertEq(recipient.balance, 0.1 ether);
    }
}
