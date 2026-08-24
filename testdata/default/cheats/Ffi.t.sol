// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract FfiTest is Test {
    function testFfi() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "bash";
        inputs[1] = "-c";
        inputs[2] =
            "echo -n 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000966666920776f726b730000000000000000000000000000000000000000000000";

        bytes memory res = vm.ffi(inputs);
        (string memory output) = abi.decode(res, (string));
        assertEq(output, "ffi works", "ffi failed");
    }

    function testFfiString() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "echo";
        inputs[1] = "-n";
        inputs[2] = "gm";

        bytes memory res = vm.ffi(inputs);
        assertEq(string(res), "gm");
    }

    function testTypedFfiOutput() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "echo";
        inputs[1] = "-n";
        inputs[2] = "42";

        assertEq(vm.ffiUint(inputs), 42);
        assertEq(vm.ffiString(inputs), "42");
        assertEq(vm.ffiBytes(inputs), hex"42");

        inputs[2] = "123";
        assertEq(vm.ffiUint(inputs), 123);
        assertEq(vm.ffiString(inputs), "123");
    }

    function testFfiBytesRejectsNonHexOutput() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "echo";
        inputs[1] = "-n";
        inputs[2] = "gm";

        vm.expectRevert();
        this.ffiBytes(inputs);
    }

    function ffiBytes(string[] memory inputs) external returns (bytes memory) {
        return vm.ffiBytes(inputs);
    }
}
