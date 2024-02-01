// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract SimpleContract {
    constructor() {
        assembly {
            sstore(1, 2)
        }
    }
}

contract DumpStateTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testDumpStateCheatAccount() public {
        // Path to temporary file that is deleted after the test
        string memory path = string.concat(vm.projectRoot(), "/fixtures/Json/test_dump_state_cheat.json");

        // Define some values to set in the state using cheatcodes
        address target = address(1001);
        bytes memory bytecode = hex"11223344";
        uint256 balance = 1.2 ether;
        uint64 nonce = 45;

        vm.etch(target, bytecode);
        vm.deal(target, balance);
        vm.setNonce(target, nonce);
        vm.store(target, bytes32(uint256(0x20)), bytes32(uint256(0x40)));
        vm.store(target, bytes32(uint256(0x40)), bytes32(uint256(0x60)));

        // Write the state to disk
        vm.dumpState(path);

        string memory json = vm.readFile(path);
        string[] memory keys = vm.parseJsonKeys(json, "");
        assertEq(keys.length, 1);

        string memory key = keys[0];
        assertEq(nonce, vm.parseJsonUint(json, string.concat(".", key, ".nonce")));
        assertEq(balance, vm.parseJsonUint(json, string.concat(".", key, ".balance")));
        assertEq(bytecode, vm.parseJsonBytes(json, string.concat(".", key, ".code")));

        string[] memory slots = vm.parseJsonKeys(json, string.concat(".", key, ".storage"));
        assertEq(slots.length, 2);

        assertEq(
            bytes32(uint256(0x40)),
            vm.parseJsonBytes32(json, string.concat(".", key, ".storage.", vm.toString(bytes32(uint256(0x20)))))
        );
        assertEq(
            bytes32(uint256(0x60)),
            vm.parseJsonBytes32(json, string.concat(".", key, ".storage.", vm.toString(bytes32(uint256(0x40)))))
        );

        vm.removeFile(path);
    }

    function testDumpStateMultipleAccounts() public {
        string memory path = string.concat(vm.projectRoot(), "/fixtures/Json/test_dump_state_multiple_accounts.json");

        vm.setNonce(address(0x100), 1);
        vm.deal(address(0x200), 1 ether);
        vm.setNonce(address(0x300), 1);
        vm.store(address(0x300), bytes32(uint256(1)), bytes32(uint256(2)));
        vm.etch(address(0x400), hex"af");

        vm.dumpState(path);

        string memory json = vm.readFile(path);
        string[] memory keys = vm.parseJsonKeys(json, "");
        assertEq(keys.length, 4);

        assertEq(4, vm.parseJsonKeys(json, string.concat(".", vm.toString(address(0x100)))).length);
        assertEq(1, vm.parseJsonUint(json, string.concat(".", vm.toString(address(0x100)), ".nonce")));
        assertEq(0, vm.parseJsonUint(json, string.concat(".", vm.toString(address(0x100)), ".balance")));
        assertEq(hex"", vm.parseJsonBytes(json, string.concat(".", vm.toString(address(0x100)), ".code")));
        assertEq(0, vm.parseJsonKeys(json, string.concat(".", vm.toString(address(0x100)), ".storage")).length);

        assertEq(4, vm.parseJsonKeys(json, string.concat(".", vm.toString(address(0x200)))).length);
        assertEq(0, vm.parseJsonUint(json, string.concat(".", vm.toString(address(0x200)), ".nonce")));
        assertEq(1 ether, vm.parseJsonUint(json, string.concat(".", vm.toString(address(0x200)), ".balance")));
        assertEq(hex"", vm.parseJsonBytes(json, string.concat(".", vm.toString(address(0x200)), ".code")));
        assertEq(0, vm.parseJsonKeys(json, string.concat(".", vm.toString(address(0x200)), ".storage")).length);

        assertEq(4, vm.parseJsonKeys(json, string.concat(".", vm.toString(address(0x300)))).length);
        assertEq(1, vm.parseJsonUint(json, string.concat(".", vm.toString(address(0x300)), ".nonce")));
        assertEq(0, vm.parseJsonUint(json, string.concat(".", vm.toString(address(0x300)), ".balance")));
        assertEq(hex"", vm.parseJsonBytes(json, string.concat(".", vm.toString(address(0x300)), ".code")));
        assertEq(1, vm.parseJsonKeys(json, string.concat(".", vm.toString(address(0x300)), ".storage")).length);
        assertEq(
            2,
            vm.parseJsonUint(
                json, string.concat(".", vm.toString(address(0x300)), ".storage.", vm.toString(bytes32(uint256(1))))
            )
        );

        assertEq(4, vm.parseJsonKeys(json, string.concat(".", vm.toString(address(0x400)))).length);
        assertEq(0, vm.parseJsonUint(json, string.concat(".", vm.toString(address(0x400)), ".nonce")));
        assertEq(0, vm.parseJsonUint(json, string.concat(".", vm.toString(address(0x400)), ".balance")));
        assertEq(hex"af", vm.parseJsonBytes(json, string.concat(".", vm.toString(address(0x400)), ".code")));
        assertEq(0, vm.parseJsonKeys(json, string.concat(".", vm.toString(address(0x400)), ".storage")).length);

        vm.removeFile(path);
    }

    function testDumpStateDeployment() public {
        string memory path = string.concat(vm.projectRoot(), "/fixtures/Json/test_dump_state_deployment.json");

        SimpleContract s = new SimpleContract();
        vm.dumpState(path);

        string memory json = vm.readFile(path);
        string[] memory keys = vm.parseJsonKeys(json, "");
        assertEq(keys.length, 1);
        assertEq(address(s), vm.parseAddress(keys[0]));
        assertEq(1, vm.parseJsonKeys(json, string.concat(".", keys[0], ".storage")).length);
        assertEq(2, vm.parseJsonUint(json, string.concat(".", keys[0], ".storage.", vm.toString(bytes32(uint256(1))))));

        vm.removeFile(path);
    }

    function testDumpStateEmptyAccount() public {
        string memory path = string.concat(vm.projectRoot(), "/fixtures/Json/test_dump_state_empty_account.json");

        SimpleContract s = new SimpleContract();
        vm.etch(address(s), hex"");
        vm.resetNonce(address(s));

        vm.dumpState(path);
        string memory json = vm.readFile(path);
        string[] memory keys = vm.parseJsonKeys(json, "");
        assertEq(keys.length, 0);
    }
}
