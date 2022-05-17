// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Test is DSTest {
    uint256 public changed = 0; 
    function t(uint256 a) public returns (uint256) {
        uint256 b = 0;
        for (uint256 i; i < a; i++) {
            b += F.t2();
        }
        emit log_string("here");
        return b;
    }

    function inc() public returns (uint256) {
        changed += 1;
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

    // 1st anvil account
    address public ACCOUNT_A = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    // 2nd anvil account
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

    function deployOther() public {
        cheats.startBroadcast(ACCOUNT_A);
        Test tmptest = new Test();
        Test test = new Test();

        // won't trigger a transaction: staticcall
        test.changed();

        // won't trigger a transaction: staticcall
        require(test.echoSender() == ACCOUNT_A);

        // will trigger a transaction
        test.t(1);

        // will trigger a transaction
        test.inc();

        cheats.stopBroadcast();
        
        require(test.echoSender() == address(this));

        cheats.broadcast(ACCOUNT_B);
        Test tmptest2 = new Test();

        cheats.broadcast(ACCOUNT_B);
        // won't trigger a transaction: staticcall
        require(test.echoSender() == ACCOUNT_B);

        cheats.broadcast(ACCOUNT_B);
        // will trigger a transaction
        test.t(2);

        cheats.broadcast(ACCOUNT_B);
        // will trigger a transaction from B
        payable(ACCOUNT_A).transfer(2);

        cheats.broadcast(ACCOUNT_B);
        // will trigger a transaction
        test.inc();

        assert(test.changed() == 2); 

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

    function deployNoArgs() public {
        cheats.broadcast();
        Test test1 = new Test();
    
        cheats.startBroadcast();
        Test test2 = new Test();
        cheats.stopBroadcast();

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
    function view_me() public pure returns (uint256) {
        return 1337;
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


    function deployCreate2() public {
        cheats.startBroadcast();
        NoLink test_c2 = new NoLink{salt: bytes32(uint256(1337))}();
        assert(test_c2.view_me() == 1337); 
        NoLink test2 = new NoLink();
        cheats.stopBroadcast();
    
    }
}
