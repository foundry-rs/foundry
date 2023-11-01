// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract TryFfiTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testTryFfi() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "bash";
        inputs[1] = "-c";
        inputs[2] =
            "echo -n 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000966666920776f726b730000000000000000000000000000000000000000000000";

        Vm.FfiResult memory f = vm.tryFfi(inputs);
        (string memory output) = abi.decode(f.stdout, (string));
        assertEq(output, "ffi works", "ffi failed");
        assertEq(f.exitCode, 0, "ffi failed");
    }

    function testTryFfiFail() public {
        string[] memory inputs = new string[](2);
        inputs[0] = "ls";
        inputs[1] = "wad";

        Vm.FfiResult memory f = vm.tryFfi(inputs);
        assertTrue(f.exitCode != 0);
    }
}
