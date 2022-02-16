// Taken from:
// https://github.com/dapphub/dapptools/blob/e41b6cd9119bbd494aba1236838b859f2136696b/src/dapp-tests/pass/cheatCodes.sol
pragma solidity ^0.8.10;
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
    // Signs data, (privateKey, digest) => (v, r, s)
    function sign(uint256,bytes32) external returns (uint8,bytes32,bytes32);
    // Gets address for a given private key, (privateKey) => (address)
    function addr(uint256) external returns (address);
    // Performs a foreign function call via terminal, (stringInputs) => (result)
    function ffi(string[] calldata) external returns (bytes memory);
    // Sets the *next* call's msg.sender to be the input address
    function prank(address) external;
    // Sets all subsequent calls' msg.sender to be the input address until `stopPrank` is called
    function startPrank(address) external;
    // Sets the *next* call's msg.sender to be the input address, and the tx.origin to be the second input
    function prank(address,address) external;
    // Sets all subsequent calls' msg.sender to be the input address until `stopPrank` is called, and the tx.origin to be the second input
    function startPrank(address,address) external;
    // Resets subsequent calls' msg.sender to be `address(this)`
    function stopPrank() external;
    // Sets an address' balance, (who, newBalance)
    function deal(address, uint256) external;
    // Sets an address' code, (who, newCode)
    function etch(address, bytes calldata) external;
    // Expects an error on next call
    function expectRevert(bytes calldata) external;
    function expectRevert(bytes4) external;
    // Record all storage reads and writes
    function record() external;
    // Gets all accessed reads and write slot from a recording session, for a given address
    function accesses(address) external returns (bytes32[] memory reads, bytes32[] memory writes);
    // Prepare an expected log with (bool checkTopic1, bool checkTopic2, bool checkTopic3, bool checkData).
    // Call this function, then emit an event, then call a function. Internally after the call, we check if
    // logs were emitted in the expected order with the expected topics and data (as specified by the booleans)
    function expectEmit(bool,bool,bool,bool) external;
    // Mocks a call to an address, returning specified data.
    // Calldata can either be strict or a partial match, e.g. if you only
    // pass a Solidity selector to the expected calldata, then the entire Solidity
    // function will be mocked.
    function mockCall(address,bytes calldata,bytes calldata) external;
    // Clears all mocked calls
    function clearMockedCalls() external;
    // Expect a call to an address with the specified calldata.
    // Calldata can either be strict or a partial match
    function expectCall(address,bytes calldata) external;
    // Gets the code from an artifact file. Takes in the relative path to the json file
    function getCode(string calldata) external returns (bytes memory);
    // Labels an address in call traces
    function label(address, string calldata) external;
    // If the condition is false, discard this run's fuzz inputs and generate new ones
    function assume(bool) external;
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
        require(blockhash(block.number) != 0x0);
    }

    function testRollHash() public {
        require(blockhash(block.number) == 0x0);
        hevm.roll(5);
        bytes32 hash = blockhash(5);
        require(hash != 0x0);

        hevm.roll(10);
        require(blockhash(10) != 0x0);

        // rolling back to 5 maintains the same hash
        hevm.roll(5);
        require(blockhash(5) == hash);
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

    function test_sign_addr_digest(uint248 sk, bytes32 digest) public {
        if (sk == 0) return; // invalid key

        (uint8 v, bytes32 r, bytes32 s) = hevm.sign(sk, digest);
        address expected = hevm.addr(sk);
        address actual = ecrecover(digest, v, r, s);

        assertEq(actual, expected);
    }

    function test_sign_addr_message(uint248 sk, bytes memory message) public {
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

    function testPrankConstructor() public {
        address new_sender = address(1337);
        hevm.prank(new_sender);
        PrankConstructor prank2 = new PrankConstructor(address(1337));
        PrankConstructor prank3 = new PrankConstructor(address(this));
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

    function testPrankDual() public {
        Prank prank = new Prank();
        address new_sender = address(1337);
        address sender = msg.sender;
        hevm.prank(new_sender, new_sender);
        prank.baz(new_sender, new_sender);
        prank.baz(address(this), tx.origin);
    }

    function testPrankConstructorDual() public {
        address new_sender = address(1337);
        hevm.prank(new_sender, new_sender);
        PrankConstructorDual prank2 = new PrankConstructorDual(address(1337), address(1337));
        PrankConstructorDual prank3 = new PrankConstructorDual(address(this), tx.origin);
    }

    function testPrankStartDual() public {
        Prank prank = new Prank();
        address new_sender = address(1337);
        address sender = msg.sender;
        hevm.startPrank(new_sender, new_sender);
        prank.baz(new_sender, new_sender);
        prank.baz(new_sender, new_sender);
        hevm.stopPrank();
        prank.baz(address(this), tx.origin);
    }

    function testPrankStartComplexDual() public {
        // A -> B, B starts pranking, doesnt call stopPrank, A calls C calls D
        // C -> D would be pranked
        ComplexPrank complexPrank = new ComplexPrank();
        Prank prank = new Prank();
        complexPrank.uncompletedPrankDual();
        prank.baz(address(this), tx.origin);
        complexPrank.completePrankDual(prank);
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

    function testExpectRevertConstructor() public {
        hevm.expectRevert("Value too large Constructor");
        ExpectRevertConstructor target = new ExpectRevertConstructor(101);
        ExpectRevertConstructor target2 = new ExpectRevertConstructor(99);
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

    function testFailDanglingExpectEmit() public {
        ExpectEmit emitter = new ExpectEmit();
        // check topic 1, topic 2, and data are the same as the following emitted event
        hevm.expectEmit(true,true,false,true);
        emit Transfer(address(this), address(1337), 1337);
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

    // Test should fail because the data is different
    function testFailExpectEmitWithCall1() public {
        ExpectEmit emitter = new ExpectEmit();
        hevm.deal(address(this), 1 ether);
        hevm.expectEmit(true,true,false,true);
        emit Transfer(address(this), address(1338), 1 gwei);
        emitter.t4(payable(address(1337)), 100 gwei);
    }


    // Test should fail because, t5 doesn't emit
    function testFailExpectEmitWithCall2() public {
        ExpectEmit emitter = new ExpectEmit();
        hevm.deal(address(this), 1 ether);
        hevm.expectEmit(true,true,false,true);
        emit Transfer(address(this), address(1338), 100 gwei);
        emitter.t5(payable(address(1337)), 100 gwei);
    }

    // Test should fail if nothing is called
    // after expectRevert
    function testFailExpectRevert3() public {
        hevm.expectRevert("revert");
    }

    function testMockArbitraryCall() public {
        hevm.mockCall(address(0xbeef), abi.encode("wowee"), abi.encode("epic"));
        (bool ok, bytes memory ret) = address(0xbeef).call(abi.encode("wowee"));
        assertTrue(ok);
        assertEq(abi.decode(ret, (string)), "epic");
    }

    function testMockContract() public {
        MockMe target = new MockMe();

        // pre-mock
        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 2);

        hevm.mockCall(
            address(target),
            abi.encodeWithSelector(target.numberB.selector),
            abi.encode(10)
        );

        // post-mock
        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 10);
    }

    function testMockInner() public {
        MockMe inner = new MockMe();
        MockInner target = new MockInner(address(inner));

        // pre-mock
        assertEq(target.sum(), 3);

        hevm.mockCall(
            address(inner),
            abi.encodeWithSelector(inner.numberB.selector),
            abi.encode(9)
        );

        // post-mock
        assertEq(target.sum(), 10);
    }

    function testMockSelector() public {
        MockMe target = new MockMe();
        assertEq(target.add(5, 5), 10);

        hevm.mockCall(
            address(target),
            abi.encodeWithSelector(target.add.selector),
            abi.encode(11)
        );

        assertEq(target.add(5, 5), 11);
    }

    function testMockCalldata() public {
        MockMe target = new MockMe();
        assertEq(target.add(5, 5), 10);
        assertEq(target.add(6, 4), 10);

        hevm.mockCall(
            address(target),
            abi.encodeWithSelector(target.add.selector, 5, 5),
            abi.encode(11)
        );

        assertEq(target.add(5, 5), 11);
        assertEq(target.add(6, 4), 10);
    }

    function testClearMockedCalls() public {
        MockMe target = new MockMe();

        hevm.mockCall(
            address(target),
            abi.encodeWithSelector(target.numberB.selector),
            abi.encode(10)
        );

        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 10);

        hevm.clearMockedCalls();

        assertEq(target.numberA(), 1);
        assertEq(target.numberB(), 2);
    }

    function testExpectCallWithData() public {
        MockMe target = new MockMe();
        hevm.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector, 1, 2)
        );
        target.add(1, 2);
    }

    function testFailExpectCallWithData() public {
        MockMe target = new MockMe();
        hevm.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector, 1, 2)
        );
        target.add(3, 3);
    }

    function testExpectInnerCall() public {
        MockMe inner = new MockMe();
        MockInner target = new MockInner(address(inner));

        hevm.expectCall(
            address(inner),
            abi.encodeWithSelector(inner.numberB.selector)
        );
        target.sum();
    }

    function testFailExpectInnerCall() public {
        MockMe inner = new MockMe();
        MockInner target = new MockInner(address(inner));

        hevm.expectCall(
            address(inner),
            abi.encodeWithSelector(inner.numberB.selector)
        );

        // this function does not call inner
        target.hello();
    }

    function testExpectSelectorCall() public {
        MockMe target = new MockMe();
        hevm.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector)
        );
        target.add(5, 5);
    }

    function testFailExpectSelectorCall() public {
        MockMe target = new MockMe();
        hevm.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector)
        );
    }

    function testFailExpectCallWithMoreParameters() public {
        MockMe target = new MockMe();
        hevm.expectCall(
            address(target),
            abi.encodeWithSelector(target.add.selector, 3, 3, 3)
        );
        target.add(3, 3);
    }

    function testGetCode() public {
        bytes memory contractCode = hevm.getCode("./testdata/Contract.json");
        assertEq(
            string(contractCode),
            string(bytes(hex"608060405234801561001057600080fd5b5060b68061001f6000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c80637ddeef2414602d575b600080fd5b60336047565b604051603e91906067565b60405180910390f35b60006007905090565b6000819050919050565b6061816050565b82525050565b6000602082019050607a6000830184605a565b9291505056fea2646970667358221220521a806ba8927fda1a9b7bf0458b0a0abf456e4611953e01489bee91783418b064736f6c634300080a0033"))
        );
    }

    function testLabel() public {
        address bob = address(1337);
        hevm.label(bob, "bob");
        bob.call{value: 100}("");
    }

    function testLabelInputReturn() public {
        Label labeled = new Label();
        hevm.label(address(labeled), "MyCustomLabel");
        labeled.withInput(address(labeled));
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

contract Label {
    function withInput(address labeled) public pure returns (address) {
        return labeled;
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

contract ExpectRevertConstructor {
    constructor(uint256 a) {
        require(a < 100, "Value too large Constructor");
    }

    function a() public returns(uint256) {
        return 1;
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

    function baz(address expectedMsgSender, address expectedOrigin) public {
        require(msg.sender == expectedMsgSender, "bad prank");
        require(tx.origin == expectedOrigin, "bad prank origin");
        InnerPrank inner = new InnerPrank();
        inner.baz(address(this), expectedOrigin);
    }

    function payableBar(address expectedMsgSender) payable public {
        bar(expectedMsgSender);
    }
}

contract PrankConstructor {
    constructor(address expectedMsgSender) {
        require(msg.sender == expectedMsgSender, "bad prank");
    }

    function bar(address expectedMsgSender) public {
        require(msg.sender == expectedMsgSender, "bad prank");
        InnerPrank inner = new InnerPrank();
        inner.bar(address(this));
    }
}

contract PrankConstructorDual {
    constructor(address expectedMsgSender, address expectedOrigin) {
        require(msg.sender == expectedMsgSender, "bad prank");
        require(tx.origin == expectedOrigin, "bad prank origin");
    }

    function baz(address expectedMsgSender, address expectedOrigin) public {
        require(msg.sender == expectedMsgSender, "bad prank");
        require(tx.origin == expectedOrigin, "bad prank origin");
        InnerPrank inner = new InnerPrank();
        inner.baz(address(this), expectedOrigin);
    }
}

contract InnerPrank {
    function bar(address expectedMsgSender) public {
        require(msg.sender == expectedMsgSender, "bad prank");
    }

    function baz(address expectedMsgSender, address expectedOrigin) public {
        require(msg.sender == expectedMsgSender, "bad prank");
        require(tx.origin == expectedOrigin, "bad prank origin");
    }
}

contract ComplexPrank {
    Hevm hevm = Hevm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    function uncompletedPrank() public {
        hevm.startPrank(address(1337));
    }

    function uncompletedPrankDual() public {
        hevm.startPrank(address(1337), address(1337));
    }

    function completePrank(Prank prank) public {
        prank.bar(address(1337));
        hevm.stopPrank();
        prank.bar(address(this));
    }

    function completePrankDual(Prank prank) public {
        prank.baz(address(1337), address(1337));
        hevm.stopPrank();
        prank.baz(address(this), tx.origin);
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

    function t4(address payable to, uint256 amount) public {
        (bool success, ) = to.call{value: amount, gas: 30_000}(new bytes(0));
        emit Transfer(msg.sender, address(1337), 100 gwei);
    }

    function t5(address payable to, uint256 amount) public {
        (bool success, ) = to.call{value: amount, gas: 30_000}(new bytes(0));
    }
}

contract MockMe {
    function numberA() public returns (uint256) {
        return 1;
    }

    function numberB() public returns (uint256) {
        return 2;
    }

    function add(uint256 a, uint256 b) public returns (uint256) {
        return a + b;
    }
}

contract MockInner {
    MockMe private inner;

    constructor(address _inner) {
        inner = MockMe(_inner);
    }

    function sum() public returns (uint256) {
        return inner.numberA() + inner.numberB();
    }

    function hello() public returns (string memory) {
        return "hi";
    }
}
