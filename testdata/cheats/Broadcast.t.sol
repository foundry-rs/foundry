// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Test is DSTest {

    function t() public {
        F.t2();
        emit log_string("here");
    }
}

library F {
    function t2() public {

    }
}

contract BroadcastTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function deploy() public {
        cheats.broadcast(address(0x1337));
        Test test = new Test();

        cheats.broadcast(address(0x1338));
        test.t();     
    }

    function deployOther() public {
        cheats.broadcast(address(0x1338));
        Test test = new Test();

        cheats.broadcast(address(0x1339));
        test.t();     
    }

    function deployPanics() public {
        cheats.broadcast(address(0x1337));
        Test test = new Test();

        cheats.broadcast(address(0x1338));
        Test test2 = new Test();

        cheats.broadcast(address(0x1338));
        test.t();     
    }
}
