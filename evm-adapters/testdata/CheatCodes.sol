// Taken from:
// https://github.com/dapphub/dapptools/blob/e41b6cd9119bbd494aba1236838b859f2136696b/src/dapp-tests/pass/cheatCodes.sol
pragma solidity ^0.8.4;
pragma experimental ABIEncoderV2;

import "./DsTest.sol";

interface Hevm {
    // Set block.timestamp (newTimestamp)
    function warp(uint256) external;
    // Set block.height (newHeight)
    function roll(uint256) external;
    // Set block.basefee (newBasefee)
    function fee(uint256) external;
    // Loads a storage slot from an address (who, slot)
    function load(address,bytes32) external returns (bytes32);
    // Stores a value to an address' storage slot, (who, slot, value)
    function store(address,bytes32,bytes32) external;
    // Signs data, (privateKey, digest) => (r, v, s)
    function sign(uint256,bytes32) external returns (uint8,bytes32,bytes32);
    // Gets address for a given private key, (privateKey) => (address)
    function addr(uint256) external returns (address);
    // Performs a foreign function call via terminal, (stringInputs) => (result)
    function ffi(string[] calldata) external returns (bytes memory);
    // Sets the *next* call's msg.sender to be the input address
    function prank(address) external;
    // Sets all subsequent calls' msg.sender to be the input address until `stopPrank` is called
    function startPrank(address) external;
    // Resets subsequent calls' msg.sender to be `address(this)`
    function stopPrank() external;
    // Sets an address' balance, (who, newBalance)
    function deal(address, uint256) external;
    // Sets an address' code, (who, newCode)
    function etch(address, bytes calldata) external;
    // Expects an error on next call
    function expectRevert(bytes calldata) external;
    // Record all storage reads and writes
    function record() external;
    // Gets all accessed reads and write slot from a recording session, for a given address
    function accesses(address) external returns (bytes32[] memory reads, bytes32[] memory writes);
    // Prepare an expected log with (bool checkTopic1, bool checkTopic2, bool checkTopic3, bool checkData).
    // Call this function, then emit an event, then call a function. Internally after the call, we check if
    // logs were emited in the expected order with the expected topics and data (as specified by the booleans)
    function expectEmit(bool,bool,bool,bool) external;
}

contract HasStorage {
    uint public slot0 = 10;
}

// We add `assertEq` tests as well to ensure that our test runner checks the
// `failed` variable.
contract CheatCodes is DSTest {
    address public store = address(new HasStorage());
    Hevm constant hevm = Hevm(HEVM_ADDRESS);
    address public who = hevm.addr(1);

    // Warp

    function testWarp(uint128 jump) public {
        uint pre = block.timestamp;
        hevm.warp(block.timestamp + jump);
        require(block.timestamp == pre + jump, "warp failed");
    }

    function testWarpAssertEq(uint128 jump) public {
        uint pre = block.timestamp;
        hevm.warp(block.timestamp + jump);
        assertEq(block.timestamp, pre + jump);
    }

    function testFailWarp(uint128 jump) public {
        uint pre = block.timestamp;
        hevm.warp(block.timestamp + jump);
        require(block.timestamp == pre + jump + 1, "warp failed");
    }

    function testFailWarpAssert(uint128 jump) public {
        uint pre = block.timestamp;
        hevm.warp(block.timestamp + jump);
        assertEq(block.timestamp, pre + jump + 1);
    }

    // Fee

    // Sets the basefee
    function testFee(uint256 fee) public {
        hevm.fee(fee);
        require(block.basefee == fee);
    }

    // Roll

    // Underscore does not run the fuzz test?!
    function testRoll(uint256 jump) public {
        uint pre = block.number;
        hevm.roll(block.number + jump);
        require(block.number == pre + jump, "roll failed");
    }

    function testFailRoll(uint32 jump) public {
        uint pre = block.number;
        hevm.roll(block.number + jump);
        assertEq(block.number, pre + jump + 1);
    }

    // function prove_warp_symbolic(uint128 jump) public {
    //     test_warp_concrete(jump);
    // }


    function test_store_load_concrete(uint x) public {
        uint ten = uint(hevm.load(store, bytes32(0)));
        assertEq(ten, 10);

        hevm.store(store, bytes32(0), bytes32(x));
        uint val = uint(hevm.load(store, bytes32(0)));
        assertEq(val, x);
    }

    // function prove_store_load_symbolic(uint x) public {
    //     test_store_load_concrete(x);
    // }

    function test_sign_addr_digest(uint sk, bytes32 digest) public {
        if (sk == 0) return; // invalid key

        (uint8 v, bytes32 r, bytes32 s) = hevm.sign(sk, digest);
        address expected = hevm.addr(sk);
        address actual = ecrecover(digest, v, r, s);

        assertEq(actual, expected);
    }

    function test_sign_addr_message(uint sk, bytes memory message) public {
        test_sign_addr_digest(sk, keccak256(message));
    }

    function testFail_sign_addr(uint sk, bytes32 digest) public {
        uint badKey = sk + 1;

        (uint8 v, bytes32 r, bytes32 s) = hevm.sign(badKey, digest);
        address expected = hevm.addr(sk);
        address actual = ecrecover(digest, v, r, s);

        assertEq(actual, expected);
    }

    function testFail_addr_zero_sk() public {
        hevm.addr(0);
    }

    function test_addr() public {
        uint sk = 77814517325470205911140941194401928579557062014761831930645393041380819009408;
        address expected = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

        assertEq(hevm.addr(sk), expected);
    }

    function testFFI() public {
        string[] memory inputs = new string[](3);
        inputs[0] = "echo";
        inputs[1] = "-n";
        inputs[2] = "0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000046163616200000000000000000000000000000000000000000000000000000000";

        bytes memory res = hevm.ffi(inputs);
        (string memory output) = abi.decode(res, (string));
        assertEq(output, "acab");
    }

    function testDeal() public {
        address addr = address(1337);
        hevm.deal(addr, 1337);
        assertEq(addr.balance, 1337);
    }

    function testPrank() public {
        Prank prank = new Prank();
        address new_sender = address(1337);
        address sender = msg.sender;
        hevm.prank(new_sender);
        prank.bar(new_sender);
        prank.bar(address(this));
    }

    function testPrankStart() public {
        Prank prank = new Prank();
        address new_sender = address(1337);
        address sender = msg.sender;
        hevm.startPrank(new_sender);
        prank.bar(new_sender);
        prank.bar(new_sender);
        hevm.stopPrank();
        prank.bar(address(this));
    }

    function testPrankPayable() public {
        Prank prank = new Prank();
        uint256 ownerBalance = address(this).balance;

        address new_sender = address(1337);
        hevm.deal(new_sender, 10 ether);
        
        hevm.prank(new_sender);
        prank.payableBar{value: 1 ether}(new_sender);
        assertEq(new_sender.balance, 9 ether);

        hevm.startPrank(new_sender);
        prank.payableBar{value: 1 ether}(new_sender);
        hevm.stopPrank();
        assertEq(new_sender.balance, 8 ether);

        assertEq(ownerBalance, address(this).balance);
    }

    function testPrankStartComplex() public {
        // A -> B, B starts pranking, doesnt call stopPrank, A calls C calls D
        // C -> D would be pranked
        ComplexPrank complexPrank = new ComplexPrank();
        Prank prank = new Prank();
        complexPrank.uncompletedPrank();
        prank.bar(address(this));
        complexPrank.completePrank(prank);
    }

    function testEtch() public {
        address rewriteCode = address(1337);

        bytes memory newCode = hex"1337";
        hevm.etch(rewriteCode, newCode);
        bytes memory n_code = getCode(rewriteCode);
        assertEq(string(newCode), string(n_code));
    }

    function testExpectRevert() public {
        ExpectRevert target = new ExpectRevert();
        hevm.expectRevert("Value too large");
        target.stringErr(101);
        target.stringErr(99);
    }

    function testExpectRevertBuiltin() public {
        ExpectRevert target = new ExpectRevert();
        hevm.expectRevert(abi.encodeWithSignature("Panic(uint256)", 0x11));
        target.arithmeticErr(101);
    }

    function testExpectCustomRevert() public {
        ExpectRevert target = new ExpectRevert();
        bytes memory data = abi.encodePacked(bytes4(keccak256("InputTooLarge()")));
        hevm.expectRevert(data);
        target.customErr(101);
        target.customErr(99);
    }

    function testCalleeExpectRevert() public {
        ExpectRevert target = new ExpectRevert();
        hevm.expectRevert("Value too largeCallee");
        target.stringErrCall(101);
        target.stringErrCall(99);
    }

    function testFailExpectRevert() public {
        ExpectRevert target = new ExpectRevert();
        hevm.expectRevert("Value too large");
        target.stringErr2(101);
    }

    function testFailExpectRevert2() public {
        ExpectRevert target = new ExpectRevert();
        hevm.expectRevert("Value too large");
        target.stringErr(99);
    }

    function testRecordAccess() public {
        RecordAccess target = new RecordAccess();
        hevm.record();
        RecordAccess2 target2 = target.record();
        (bytes32[] memory reads, bytes32[] memory writes) = hevm.accesses(address(target));
        (bytes32[] memory reads2, bytes32[] memory writes2) = hevm.accesses(address(target2));
        assertEq(reads.length, 2); // sstore has to do an sload to grab the original storage, so we effectively have 2 sloads
        assertEq(writes.length, 1);
        assertEq(reads[0], bytes32(uint256(1)));
        assertEq(writes[0], bytes32(uint256(1)));
        assertEq(reads2.length, 2); // sstore has to do an sload to grab the original storage, so we effectively have 2 sloads
        assertEq(writes2.length, 1);
        assertEq(reads2[0], bytes32(uint256(2)));
        assertEq(writes2[0], bytes32(uint256(2)));
    }

    event Transfer(address indexed from,address indexed to, uint256 amount);
    function testExpectEmit() public {
        ExpectEmit emitter = new ExpectEmit();
        // check topic 1, topic 2, and data are the same as the following emitted event
        hevm.expectEmit(true,true,false,true);
        emit Transfer(address(this), address(1337), 1337);
        emitter.t();
    }

    function testExpectEmitMultiple() public {
        ExpectEmit emitter = new ExpectEmit();
        hevm.expectEmit(true,true,false,true);
        emit Transfer(address(this), address(1337), 1337);
        hevm.expectEmit(true,true,false,true);
        emit Transfer(address(this), address(1337), 1337);
        emitter.t3();
    }

    function testExpectEmit2() public {
        ExpectEmit emitter = new ExpectEmit();
        hevm.expectEmit(true,false,false,true);
        emit Transfer(address(this), address(1338), 1337);
        emitter.t();
    }

    function testExpectEmit3() public {
        ExpectEmit emitter = new ExpectEmit();
        hevm.expectEmit(false,false,false,true);
        emit Transfer(address(1338), address(1338), 1337);
        emitter.t();
    }

    function testExpectEmit4() public {
        ExpectEmit emitter = new ExpectEmit();
        hevm.expectEmit(true,true,true,false);
        emit Transfer(address(this), address(1337), 1338);
        emitter.t();
    }

    function testExpectEmit5() public {
        ExpectEmit emitter = new ExpectEmit();
        hevm.expectEmit(true,true,true,false);
        emit Transfer(address(this), address(1337), 1338);
        emitter.t2();
    }

    function testFailExpectEmit() public {
        ExpectEmit emitter = new ExpectEmit();
        hevm.expectEmit(true,true,false,true);
        emit Transfer(address(this), address(1338), 1337);
        emitter.t();
    }

    // Test should fail if nothing is called
    // after expectRevert
    function testFailExpectRevert3() public {
        hevm.expectRevert("revert");
    }  

    function getCode(address who) internal returns (bytes memory o_code) {
        assembly {
            // retrieve the size of the code, this needs assembly
            let size := extcodesize(who)
            // allocate output byte array - this could also be done without assembly
            // by using o_code = new bytes(size)
            o_code := mload(0x40)
            // new "memory end" including padding
            mstore(0x40, add(o_code, and(add(add(size, 0x20), 0x1f), not(0x1f))))
            // store length in memory
            mstore(o_code, size)
            // actually retrieve the code, this needs assembly
            extcodecopy(who, add(o_code, 0x20), 0, size)
        }
    }
}

contract RecordAccess {
    function record() public returns (RecordAccess2) {
        assembly {
            sstore(1, add(sload(1), 1))
        }
        RecordAccess2 target2 = new RecordAccess2();
        target2.record();
        return target2;
    }
}

contract RecordAccess2 {
    function record() public {
        assembly {
            sstore(2, add(sload(2), 1))
        }
    }
}

error InputTooLarge();
contract ExpectRevert {
    function stringErrCall(uint256 a) public returns (uint256) {
        ExpectRevertCallee callee = new ExpectRevertCallee();
        uint256 amount = callee.stringErr(a);
        return amount;
    }

    function stringErr(uint256 a) public returns (uint256) {
        require(a < 100, "Value too large");
        return a;
    }

    function arithmeticErr(uint256 a) public returns (uint256) {
        uint256 b = 100 - a;
        return b;
    }

    function stringErr2(uint256 a) public returns (uint256) {
        require(a < 100, "Value too large2");
        return a;
    }

    function customErr(uint256 a) public returns (uint256) {
        if (a > 99) {
            revert InputTooLarge();
        }
        return a;
    }
}

contract ExpectRevertCallee {
    function stringErr(uint256 a) public returns (uint256) {
        require(a < 100, "Value too largeCallee");
        return a;
    }

    function stringErr2(uint256 a) public returns (uint256) {
        require(a < 100, "Value too large2Callee");
        return a;
    }
}

contract Prank {
    function bar(address expectedMsgSender) public {
        require(msg.sender == expectedMsgSender, "bad prank");
        InnerPrank inner = new InnerPrank();
        inner.bar(address(this));
    }

    function payableBar(address expectedMsgSender) payable public {
        bar(expectedMsgSender);
    }
}

contract InnerPrank {
    function bar(address expectedMsgSender) public {
        require(msg.sender == expectedMsgSender, "bad prank");
    }
}

contract ComplexPrank {
    Hevm hevm = Hevm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    function uncompletedPrank() public {
        hevm.startPrank(address(1337));
    }

    function completePrank(Prank prank) public {
        prank.bar(address(1337));
        hevm.stopPrank();
        prank.bar(address(this));
    }
}

contract ExpectEmit {
    event Transfer(address indexed from,address indexed to, uint256 amount);
    event Transfer2(address indexed from,address indexed to, uint256 amount);
    function t() public {
        emit Transfer(msg.sender, address(1337), 1337);
    }

    function t2() public {
        emit Transfer2(msg.sender, address(1337), 1337);
        emit Transfer(msg.sender, address(1337), 1337);
    }

    function t3() public {
        emit Transfer(msg.sender, address(1337), 1337);
        emit Transfer(msg.sender, address(1337), 1337);
    }
}

