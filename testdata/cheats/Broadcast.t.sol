// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Test is DSTest {

    function t() public {
        emit log_string("here");
    }
}

contract BroadcastTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testDeploy() public {
        cheats.broadcast(address(0x1337));
        Test test = new Test();

        cheats.broadcast(address(0x1338));
        test.t();     
    }
}
