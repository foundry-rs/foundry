// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract FfiTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testFfi() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "bash";
        inputs[1] = "-c";
        inputs[2] =
            "echo -n 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000966666920776f726b730000000000000000000000000000000000000000000000";

        bytes memory res = cheats.ffi(inputs);
        (string memory output) = abi.decode(res, (string));
        assertEq(output, "ffi works", "ffi failed");
    }

    function testFfiString() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "echo";
        inputs[1] = "-n";
        inputs[2] = "gm";

        bytes memory res = cheats.ffi(inputs);
        assertEq(string(res), "gm");
    }
}
