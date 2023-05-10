// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

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

    function multiple_arguments(uint256 a, address b, uint256[] memory c) public returns (uint256) {}

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

    function deployPrivateKey() public {
        string memory mnemonic = "test test test test test test test test test test test junk";

        uint256 privateKey = cheats.deriveKey(mnemonic, 3);
        assertEq(privateKey, 0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6);

        cheats.broadcast(privateKey);
        Test test = new Test();

        cheats.startBroadcast(privateKey);
        Test test2 = new Test();
        cheats.stopBroadcast();
    }

    function deployRememberKey() public {
        string memory mnemonic = "test test test test test test test test test test test junk";

        uint256 privateKey = cheats.deriveKey(mnemonic, 3);
        assertEq(privateKey, 0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6);

        address thisAddress = cheats.rememberKey(privateKey);
        assertEq(thisAddress, 0x90F79bf6EB2c4f870365E785982E1f101E93b906);

        cheats.broadcast(thisAddress);
        Test test = new Test();
    }

    function deployRememberKeyResume() public {
        cheats.broadcast(ACCOUNT_A);
        Test test = new Test();

        string memory mnemonic = "test test test test test test test test test test test junk";

        uint256 privateKey = cheats.deriveKey(mnemonic, 3);
        address thisAddress = cheats.rememberKey(privateKey);

        cheats.broadcast(thisAddress);
        Test test2 = new Test();
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

    function testFailNoBroadcast() public {
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

interface INoLink {
    function t(uint256 a) external returns (uint256);
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

    function deployMany() public {
        assert(cheats.getNonce(msg.sender) == 0);

        cheats.startBroadcast();

        for (uint256 i; i < 25; i++) {
            NoLink test9 = new NoLink();
        }

        cheats.stopBroadcast();
    }

    function deployCreate2() public {
        cheats.startBroadcast();
        NoLink test_c2 = new NoLink{salt: bytes32(uint256(1337))}();
        assert(test_c2.view_me() == 1337);
        NoLink test2 = new NoLink();
        cheats.stopBroadcast();
    }

    function errorStaticCall() public {
        cheats.broadcast();
        NoLink test11 = new NoLink();

        cheats.broadcast();
        test11.view_me();
    }
}

contract BroadcastMix is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    // ganache-cli -d 1st
    address public ACCOUNT_A = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

    // ganache-cli -d 2nd
    address public ACCOUNT_B = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8;

    function more() internal {
        cheats.broadcast();
        NoLink test11 = new NoLink();
    }

    function deployMix() public {
        address user = msg.sender;
        assert(user == address(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266));

        NoLink no = new NoLink();

        cheats.startBroadcast();
        NoLink test1 = new NoLink();
        test1.t(2);
        NoLink test2 = new NoLink();
        test2.t(2);
        cheats.stopBroadcast();

        cheats.startBroadcast(user);
        NoLink test3 = new NoLink();
        NoLink test4 = new NoLink();
        test4.t(2);
        cheats.stopBroadcast();

        cheats.broadcast();
        test4.t(2);

        cheats.broadcast();
        NoLink test5 = new NoLink();

        cheats.broadcast();
        INoLink test6 = INoLink(address(new NoLink()));

        cheats.broadcast();
        NoLink test7 = new NoLink();

        cheats.broadcast(user);
        NoLink test8 = new NoLink();

        cheats.broadcast();
        NoLink test9 = new NoLink();

        cheats.broadcast(user);
        NoLink test10 = new NoLink();

        more();
    }
}

contract BroadcastTestSetup is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function setUp() public {
        // It predeployed a library first
        assert(cheats.getNonce(msg.sender) == 1);

        cheats.broadcast();
        Test t = new Test();

        cheats.broadcast();
        t.t(2);
    }

    function run() public {
        cheats.broadcast();
        new NoLink();

        cheats.broadcast();
        Test t = new Test();

        cheats.broadcast();
        t.t(3);
    }
}

contract BroadcastTestLog is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function run() public {
        uint256[] memory arr = new uint256[](2);
        arr[0] = 3;
        arr[1] = 4;

        cheats.startBroadcast();
        {
            Test c1 = new Test();
            Test c2 = new Test{salt: bytes32(uint256(1337))}();

            c1.multiple_arguments(1, address(0x1337), arr);
            c1.inc();
            c2.t(1);

            payable(address(0x1337)).transfer(0.0001 ether);
        }
        cheats.stopBroadcast();
    }
}

contract TestInitialBalance is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function runCustomSender() public {
        // Make sure we're testing a different caller than the default one.
        assert(msg.sender != address(0x00a329c0648769A73afAc7F9381E08FB43dBEA72));

        // NodeConfig::test() sets the balance of the address used in this test to 100 ether.
        assert(msg.sender.balance == 100 ether);

        cheats.broadcast();
        new NoLink();
    }

    function runDefaultSender() public {
        // Make sure we're testing with the default caller.
        assert(msg.sender == address(0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38));

        assert(msg.sender.balance == type(uint256).max);

        cheats.broadcast();
        new NoLink();
    }
}

contract MultiChainBroadcastNoLink is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    // ganache-cli -d 1st
    address public ACCOUNT_A = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

    // ganache-cli -d 2nd
    address public ACCOUNT_B = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8;

    function deploy(string memory sforkA, string memory sforkB) public {
        uint256 forkA = cheats.createFork(sforkA);
        uint256 forkB = cheats.createFork(sforkB);

        cheats.selectFork(forkA);
        cheats.broadcast(address(ACCOUNT_A));
        new NoLink();
        cheats.broadcast(address(ACCOUNT_B));
        new NoLink();
        cheats.selectFork(forkB);
        cheats.startBroadcast(address(ACCOUNT_B));
        new NoLink();
        new NoLink();
        new NoLink();
        cheats.stopBroadcast();
        cheats.startBroadcast(address(ACCOUNT_A));
        new NoLink();
        new NoLink();
    }

    function deployError(string memory sforkA, string memory sforkB) public {
        uint256 forkA = cheats.createFork(sforkA);
        uint256 forkB = cheats.createFork(sforkB);

        cheats.selectFork(forkA);
        cheats.broadcast(address(ACCOUNT_A));
        new NoLink();
        cheats.startBroadcast(address(ACCOUNT_B));
        new NoLink();

        cheats.selectFork(forkB);
        cheats.broadcast(address(ACCOUNT_B));
        new NoLink();
    }
}

contract MultiChainBroadcastLink is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    // ganache-cli -d 1st
    address public ACCOUNT_A = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

    // ganache-cli -d 2nd
    address public ACCOUNT_B = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8;

    function deploy(string memory sforkA, string memory sforkB) public {
        uint256 forkA = cheats.createFork(sforkA);
        uint256 forkB = cheats.createFork(sforkB);

        cheats.selectFork(forkA);
        cheats.broadcast(address(ACCOUNT_B));
        new Test();

        cheats.selectFork(forkB);
        cheats.broadcast(address(ACCOUNT_B));
        new Test();
    }
}

contract BroadcastEmptySetUp is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function setUp() public {}

    function run() public {
        cheats.broadcast();
        new Test();
    }
}

contract ContractA {
    uint256 var1;

    constructor(address script_caller) {
        require(msg.sender == script_caller);
        require(tx.origin == script_caller);
    }

    function method(address script_caller) public {
        require(msg.sender == script_caller);
        require(tx.origin == script_caller);
    }
}

contract ContractB {
    uint256 var2;

    constructor(address script_caller) {
        require(address(0x1337) != script_caller);
        require(msg.sender == address(0x1337));
        require(tx.origin == address(0x1337));
    }

    function method(address script_caller) public {
        require(address(0x1337) != script_caller);
        require(msg.sender == address(0x1337));
        require(tx.origin == address(0x1337));
    }
}

contract CheckOverrides is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function run() external {
        // `script_caller` can be set by `--private-key ...` or `--sender ...`
        // Otherwise it will take the default value of 0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38
        address script_caller = msg.sender;
        require(script_caller == 0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38);
        require(tx.origin == script_caller);

        // startBroadcast(script_caller)
        cheats.startBroadcast();
        require(tx.origin == script_caller);
        require(msg.sender == script_caller);

        ContractA a = new ContractA(script_caller);
        require(tx.origin == script_caller);
        require(msg.sender == script_caller);

        a.method(script_caller);
        require(tx.origin == script_caller);
        require(msg.sender == script_caller);

        cheats.stopBroadcast();

        // startBroadcast(msg.sender)
        cheats.startBroadcast(address(0x1337));
        require(tx.origin == script_caller);
        require(msg.sender == script_caller);
        require(msg.sender != address(0x1337));

        ContractB b = new ContractB(script_caller);
        require(tx.origin == script_caller);
        require(msg.sender == script_caller);

        b.method(script_caller);
        require(tx.origin == script_caller);
        require(msg.sender == script_caller);

        cheats.stopBroadcast();
    }
}
