// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/7481
// This test ensures that we don't panic
contract Issue7481Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testFailTransact() public {
        vm.createSelectFork("mainnet", 19514903);

        // Transfer some funds to sender of tx being transacted to ensure that it appears in journaled state
        payable(address(0x5C60cD7a3D50877Bfebd484750FBeb245D936dAD)).call{value: 1}("");
        vm.transact(0xccfd66fc409a633a99b5b75b0e9a2040fcf562d03d9bee3fefc1a5c0eb49c999);

        // Revert the current call to ensure that revm can revert state journal
        revert("HERE");
    }
}
