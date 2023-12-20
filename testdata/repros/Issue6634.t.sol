// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

contract Box {
    uint256 public number;

    constructor(uint256 _number) {
        number = _number;
    }
}

// https://github.com/foundry-rs/foundry/issues/6634
contract Issue6634Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test() public {
        address CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

        vm.startStateDiffRecording();
        Box a = new Box{salt: 0}(1);

        Vm.AccountAccess[] memory called = vm.stopAndReturnStateDiff();
        assertEq(called.length, 2, "incorrect length");
        assertEq(uint256(called[0].kind), uint256(Vm.AccountAccessKind.Call), "first AccountAccess is incorrect kind");
        assertEq(called[0].account, CREATE2_DEPLOYER, "first AccountAccess accout is incorrect");
        assertEq(
            uint256(called[1].kind), uint256(Vm.AccountAccessKind.Create), "second AccountAccess is incorrect kind"
        );
        assertEq(called[1].accessor, CREATE2_DEPLOYER, "second AccountAccess accessor is incorrect");
        assertEq(called[1].account, address(a), "first AccountAccess accout is incorrect");
    }
}
