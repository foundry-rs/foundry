// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Test is DSTest {
    function t(uint256 a) public returns (uint256) {
        uint256 b = 0;
        for (uint256 i; i < a; i++) {
            b += F.t2();
        }
        emit log_string("here");
        return b;
    }
}

library F {
    function t2() public view returns (uint256) {
        return 1;
    }
}

contract BroadcastTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function deploy() public {
        cheats.broadcast(address(0x1337));
        Test test = new Test();

        // this wont generate tx to sign
        uint256 b = test.t(4);

        // this will
        cheats.broadcast(address(0x1338));
        test.t(b);     
    }

    function deployOther() public {
        cheats.broadcast(address(0x1338));
        Test test = new Test();

        cheats.broadcast(address(0x1339));
        test.t(0);     
    }

    function deployPanics() public {
        cheats.broadcast(address(0x1337));
        Test test = new Test();

        cheats.broadcast(address(0x1338));
        Test test2 = new Test();

        cheats.broadcast(address(0x1338));
        test.t(0);     
    }
}
