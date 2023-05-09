// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract TryFfiTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testTryFfi() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "bash";
        inputs[1] = "-c";
        inputs[2] =
            "echo -n 0x0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000966666920776f726b730000000000000000000000000000000000000000000000";

        Cheats.FfiResult memory f = cheats.tryFfi(inputs);
        // (string memory output) = abi.decode(f.stdout, (string));
        // assertEq(output, "ffi works", "ffi failed");
        assertEq(f.exit_code, 0, "ffi failed");
    }

    function testTryFfiFail() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "bash";
        inputs[1] = "-c";
        inputs[2] =
            "ls foo";

        Cheats.FfiResult memory f = cheats.tryFfi(inputs);
        // assert(f.exit_code != 0);
        // assertEq(string(f.stderr), string("command not found"));
    }
}
