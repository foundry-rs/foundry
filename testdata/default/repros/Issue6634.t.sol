// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";
import "../logs/console.sol";

contract Box {
    uint256 public number;

    constructor(uint256 _number) {
        number = _number;
    }
}

// https://github.com/foundry-rs/foundry/issues/6634
contract Issue6634Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_Create2FactoryCallRecordedInStandardTest() public {
        address CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

        vm.startStateDiffRecording();
        Box a = new Box{salt: 0}(1);

        Vm.AccountAccess[] memory called = vm.stopAndReturnStateDiff();
        address addr = vm.computeCreate2Address(
            0, keccak256(abi.encodePacked(type(Box).creationCode, uint256(1))), address(CREATE2_DEPLOYER)
        );
        assertEq(addr, called[1].account, "state diff contract address is not correct");
        assertEq(address(a), called[1].account, "returned address is not correct");

        assertEq(called.length, 2, "incorrect length");
        assertEq(uint256(called[0].kind), uint256(Vm.AccountAccessKind.Call), "first AccountAccess is incorrect kind");
        assertEq(called[0].account, CREATE2_DEPLOYER, "first AccountAccess account is incorrect");
        assertEq(called[0].accessor, address(this), "first AccountAccess accessor is incorrect");
        assertEq(
            uint256(called[1].kind), uint256(Vm.AccountAccessKind.Create), "second AccountAccess is incorrect kind"
        );
        assertEq(called[1].accessor, CREATE2_DEPLOYER, "second AccountAccess accessor is incorrect");
        assertEq(called[1].account, address(a), "second AccountAccess account is incorrect");
    }

    function test_Create2FactoryCallRecordedWhenPranking() public {
        address CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
        address accessor = address(0x5555);

        vm.startPrank(accessor);
        vm.startStateDiffRecording();
        Box a = new Box{salt: 0}(1);

        Vm.AccountAccess[] memory called = vm.stopAndReturnStateDiff();
        address addr = vm.computeCreate2Address(
            0, keccak256(abi.encodePacked(type(Box).creationCode, uint256(1))), address(CREATE2_DEPLOYER)
        );
        assertEq(addr, called[1].account, "state diff contract address is not correct");
        assertEq(address(a), called[1].account, "returned address is not correct");

        assertEq(called.length, 2, "incorrect length");
        assertEq(uint256(called[0].kind), uint256(Vm.AccountAccessKind.Call), "first AccountAccess is incorrect kind");
        assertEq(called[0].account, CREATE2_DEPLOYER, "first AccountAccess account is incorrect");
        assertEq(called[0].accessor, accessor, "first AccountAccess accessor is incorrect");
        assertEq(
            uint256(called[1].kind), uint256(Vm.AccountAccessKind.Create), "second AccountAccess is incorrect kind"
        );
        assertEq(called[1].accessor, CREATE2_DEPLOYER, "second AccountAccess accessor is incorrect");
        assertEq(called[1].account, address(a), "second AccountAccess account is incorrect");
    }

    function test_Create2FactoryCallRecordedWhenBroadcasting() public {
        address CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;
        address accessor = address(0x5555);

        vm.startBroadcast(accessor);
        vm.startStateDiffRecording();
        Box a = new Box{salt: 0}(1);

        Vm.AccountAccess[] memory called = vm.stopAndReturnStateDiff();
        address addr = vm.computeCreate2Address(
            0, keccak256(abi.encodePacked(type(Box).creationCode, uint256(1))), address(CREATE2_DEPLOYER)
        );
        assertEq(addr, called[1].account, "state diff contract address is not correct");
        assertEq(address(a), called[1].account, "returned address is not correct");

        assertEq(called.length, 2, "incorrect length");
        assertEq(uint256(called[0].kind), uint256(Vm.AccountAccessKind.Call), "first AccountAccess is incorrect kind");
        assertEq(called[0].account, CREATE2_DEPLOYER, "first AccountAccess account is incorrect");
        assertEq(called[0].accessor, accessor, "first AccountAccess accessor is incorrect");
        assertEq(
            uint256(called[1].kind), uint256(Vm.AccountAccessKind.Create), "second AccountAccess is incorrect kind"
        );
        assertEq(called[1].accessor, CREATE2_DEPLOYER, "second AccountAccess accessor is incorrect");
        assertEq(called[1].account, address(a), "second AccountAccess account is incorrect");
    }
}
