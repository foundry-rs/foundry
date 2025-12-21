// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract GetRawBlockHeaderTest is Test {
    function testGetRawBlockHeaderWithFork() public {
        vm.createSelectFork("mainnet");
        assertEq(
            keccak256(vm.getRawBlockHeader(22985278)),
            // `cast keccak256 $(cast block 22985278 --raw)`
            0x492419d85d2817f50577807a287742fbdcaae00ce89f2ea885e419ee4493b00f
        );
    }
}
