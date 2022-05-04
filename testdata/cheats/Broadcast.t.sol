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

    function echoSender() public view returns (address) {
        return msg.sender;
    } 
}

library F {
    function t2() public pure returns (uint256) {
        return 1;
    }
}

contract BroadcastTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    // ganache-cli -d 1st
    address public ACCOUNT_A = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    // ganache-cli -d 2nd
    address public ACCOUNT_B = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8;

    function deploy() public {
        cheats.broadcast(ACCOUNT_A);
        Test test = new Test();

        // this wont generate tx to sign
        uint256 b = test.t(4);

        // this will
        cheats.broadcast(ACCOUNT_B);
        test.t(2);     
    }

    function deployWithResume() public {
        cheats.broadcast(ACCOUNT_B);
        Test test = new Test();

        // this wont generate tx to sign
        uint256 b = test.t(5);

        // this will
        cheats.broadcast(ACCOUNT_A);
        test.t(b);     
    }

    function deployDefault() public {
        cheats.broadcast();
        Test test = new Test();

        // this wont generate tx to sign
        uint256 b = test.t(4);

        // this will
        cheats.broadcast(address(0x1338));
        test.t(b);     
    }

    function deployOther() public {
        cheats.startBroadcast(address(0xb1eF51983621Adb0AF040Da515d6c04fe7546753));
        Test test = new Test();
        require(test.echoSender() == address(0xb1eF51983621Adb0AF040Da515d6c04fe7546753));
        cheats.stopBroadcast();
        require(test.echoSender() == address(this));

        cheats.broadcast(address(0xb1eF51983621Adb0AF040Da515d6c04fe7546753));
        require(test.echoSender() == address(0xb1eF51983621Adb0AF040Da515d6c04fe7546753));
    }

    function deployPanics() public {

        cheats.broadcast(address(0x1337));
        Test test = new Test();

        // This panics because this would cause an additional relinking that isnt conceptually correct
        // from a solidity standpoint. Basically, this contract `BroadcastTest`, injects the code of
        // `Test` *into* its code. So it isn't reasonable to break solidity to our will of having *two*
        // versions of `Test` based on the sender/linker.
        cheats.broadcast(address(0x1338));
        new Test();

        cheats.broadcast(address(0x1338));
        test.t(0);     
    }
}


contract NoLink is DSTest {
    function t(uint256 a) public returns (uint256) {
        uint256 b = 0;
        for (uint256 i; i < a; i++) {
            b += i;
        }
        emit log_string("here");
        return b;
    }
}

contract BroadcastTestNoLinking is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    // ganache-cli -d 1st
    address public ACCOUNT_A = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

    // ganache-cli -d 2nd
    address public ACCOUNT_B = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8;

    function deployDoesntPanic() public {
        cheats.broadcast(address(ACCOUNT_A));
        NoLink test = new NoLink();

        cheats.broadcast(address(ACCOUNT_B));
        new NoLink();

        cheats.broadcast(address(ACCOUNT_B));
        test.t(0);     
    }
}
