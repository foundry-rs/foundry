// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "./test.sol";
import "./Vm.sol";

contract Emitter {
    uint256 public thing;

    event Something(uint256 indexed topic1, uint256 indexed topic2, uint256 indexed topic3, uint256 data);
    event A(uint256 indexed topic1);
    event B(uint256 indexed topic1);
    event C(uint256 indexed topic1);
    event D(uint256 indexed topic1);
    event E(uint256 indexed topic1);

    /// This event has 0 indexed topics, but the one in our tests
    /// has exactly one indexed topic. Even though both of these
    /// events have the same topic 0, they are different and should
    /// be non-comparable.
    ///
    /// Ref: issue #760
    event SomethingElse(uint256 data);

    event SomethingNonIndexed(uint256 data);

    function emitEvent(uint256 topic1, uint256 topic2, uint256 topic3, uint256 data) public {
        emit Something(topic1, topic2, topic3, data);
    }

    function emitNEvents(uint256 topic1, uint256 topic2, uint256 topic3, uint256 data, uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            emit Something(topic1, topic2, topic3, data);
        }
    }

    function emitMultiple(
        uint256[2] memory topic1,
        uint256[2] memory topic2,
        uint256[2] memory topic3,
        uint256[2] memory data
    ) public {
        emit Something(topic1[0], topic2[0], topic3[0], data[0]);
        emit Something(topic1[1], topic2[1], topic3[1], data[1]);
    }

    function emitAndNest() public {
        emit Something(1, 2, 3, 4);
        emitNested(Emitter(address(this)), 1, 2, 3, 4);
    }

    function emitOutOfExactOrder() public {
        emit SomethingNonIndexed(1);
        emit Something(1, 2, 3, 4);
        emit Something(1, 2, 3, 4);
        emit Something(1, 2, 3, 4);
    }

    function emitNested(Emitter inner, uint256 topic1, uint256 topic2, uint256 topic3, uint256 data) public {
        inner.emitEvent(topic1, topic2, topic3, data);
    }

    function getVar() public pure returns (uint256) {
        return 1;
    }

    /// Used to test matching of consecutive different events,
    /// even if they're not emitted right after the other.
    function emitWindow() public {
        emit A(1);
        emit B(2);
        emit C(3);
        emit D(4);
        emit E(5);
    }

    function emitNestedWindow() public {
        emit A(1);
        emit C(3);
        emit E(5);
        this.emitWindow();
    }

    // Used to test matching of consecutive different events
    // split across subtree calls.
    function emitSplitWindow() public {
        this.emitWindow();
        this.emitWindow();
    }

    function emitWindowAndOnTest(ExpectEmitFailureTest t) public {
        this.emitWindow();
        t.emitLocal();
    }

    /// Ref: issue #1214
    function doesNothing() public pure {}

    function changeThing(uint256 num) public {
        thing = num;
    }

    /// Ref: issue #760
    function emitSomethingElse(uint256 data) public {
        emit SomethingElse(data);
    }
}

/// Emulates `Emitter` in #760
contract LowLevelCaller {
    function f() external {
        address(this).call(abi.encodeWithSignature("g()"));
    }

    function g() public {}
}

contract ExpectEmitFailureTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Emitter emitter;

    event Something(uint256 indexed topic1, uint256 indexed topic2, uint256 indexed topic3, uint256 data);

    event SomethingElse(uint256 indexed topic1);

    event A(uint256 indexed topic1);
    event B(uint256 indexed topic1);
    event C(uint256 indexed topic1);
    event D(uint256 indexed topic1);
    event E(uint256 indexed topic1);

    function setUp() public {
        emitter = new Emitter();
    }

    function emitLocal() public {
        emit A(1);
    }

    function testShouldFailExpectEmitDanglingNoReference() public {
        vm.expectEmit(false, false, false, false);
    }

    function testShouldFailExpectEmitDanglingWithReference() public {
        vm.expectEmit(false, false, false, false);
        emit Something(1, 2, 3, 4);
    }

    /// The topics that are checked are altered to be incorrect
    /// compared to the reference.
    function testShouldFailExpectEmit(
        bool checkTopic1,
        bool checkTopic2,
        bool checkTopic3,
        bool checkData,
        uint128 topic1,
        uint128 topic2,
        uint128 topic3,
        uint128 data
    ) public {
        vm.assume(checkTopic1 || checkTopic2 || checkTopic3 || checkData);

        uint256 transformedTopic1 = checkTopic1 ? uint256(topic1) + 1 : uint256(topic1);
        uint256 transformedTopic2 = checkTopic2 ? uint256(topic2) + 1 : uint256(topic2);
        uint256 transformedTopic3 = checkTopic3 ? uint256(topic3) + 1 : uint256(topic3);
        uint256 transformedData = checkData ? uint256(data) + 1 : uint256(data);

        vm.expectEmit(checkTopic1, checkTopic2, checkTopic3, checkData);

        emit Something(topic1, topic2, topic3, data);
        emitter.emitEvent(transformedTopic1, transformedTopic2, transformedTopic3, transformedData);
    }

    /// The topics that are checked are altered to be incorrect
    /// compared to the reference.
    function testShouldFailExpectEmitNested(
        bool checkTopic1,
        bool checkTopic2,
        bool checkTopic3,
        bool checkData,
        uint128 topic1,
        uint128 topic2,
        uint128 topic3,
        uint128 data
    ) public {
        vm.assume(checkTopic1 || checkTopic2 || checkTopic3 || checkData);
        Emitter inner = new Emitter();

        uint256 transformedTopic1 = checkTopic1 ? uint256(topic1) + 1 : uint256(topic1);
        uint256 transformedTopic2 = checkTopic2 ? uint256(topic2) + 1 : uint256(topic2);
        uint256 transformedTopic3 = checkTopic3 ? uint256(topic3) + 1 : uint256(topic3);
        uint256 transformedData = checkData ? uint256(data) + 1 : uint256(data);

        vm.expectEmit(checkTopic1, checkTopic2, checkTopic3, checkData);

        emit Something(topic1, topic2, topic3, data);
        emitter.emitNested(inner, transformedTopic1, transformedTopic2, transformedTopic3, transformedData);
    }

    function testShouldFailExpectEmitCanMatchWithoutExactOrder() public {
        vm.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);
        // This should fail, as this event is never emitted
        // in between the other two Something events.
        vm.expectEmit(true, true, true, true);
        emit SomethingElse(1);
        vm.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);

        emitter.emitOutOfExactOrder();
    }

    function testShouldFailExpectEmitAddress() public {
        vm.expectEmit(address(0));
        emit Something(1, 2, 3, 4);

        emitter.emitEvent(1, 2, 3, 4);
    }

    function testShouldFailExpectEmitAddressWithArgs() public {
        vm.expectEmit(true, true, true, true, address(0));
        emit Something(1, 2, 3, 4);

        emitter.emitEvent(1, 2, 3, 4);
    }

    /// Ref: issue #760
    function testShouldFailLowLevelWithoutEmit() public {
        LowLevelCaller caller = new LowLevelCaller();

        vm.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);

        // This does not emit an event, so this test should fail
        caller.f();
    }

    function testShouldFailNoEmitDirectlyOnNextCall() public {
        LowLevelCaller caller = new LowLevelCaller();

        vm.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);

        // This call does not emit. As emit expects the next call to emit, this should fail.
        caller.f();
        // This call does emit, but it is a call later than expected.
        emitter.emitEvent(1, 2, 3, 4);
    }

    /// Ref: issue #760
    function testShouldFailDifferentIndexedParameters() public {
        vm.expectEmit(true, false, false, false);
        emit SomethingElse(1);

        // This should fail since `SomethingElse` in the test
        // and in the `Emitter` contract have differing
        // amounts of indexed topics.
        emitter.emitSomethingElse(1);
    }

    /// This test should fail, as the call to `changeThing` is not a static call.
    /// While we can ignore static calls, we cannot ignore normal calls.
    function testShouldFailEmitOnlyAppliesToNextCall() public {
        vm.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);
        // This works because it's a staticcall.
        emitter.doesNothing();
        // This should make the test fail as it's a normal call.
        emitter.changeThing(block.timestamp);

        emitter.emitEvent(1, 2, 3, 4);
    }

    /// emitWindow() emits events A, B, C, D, E.
    /// We should not be able to match [B, A, C, D, E] as B and A are flipped.
    function testShouldFailCanMatchConsecutiveEvents() public {
        vm.expectEmit(true, false, false, true);
        emit B(2);
        vm.expectEmit(true, false, false, true);
        emit A(1);
        vm.expectEmit(true, false, false, true);
        emit C(3);
        vm.expectEmit(true, false, false, true);
        emit D(4);
        vm.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitWindow();
    }

    /// emitWindowNested() emits events A, C, E, A, B, C, D, E, the last 5 on an external call.
    /// We should NOT be able to match [A, A, E, E], as while we're matching the correct amount
    /// of events, they're not in the correct order. It should be [A, E, A, E].
    function testShouldFailMatchRepeatedEventsOutOfOrder() public {
        vm.expectEmit(true, false, false, true);
        emit A(1);
        vm.expectEmit(true, false, false, true);
        emit A(1);
        vm.expectEmit(true, false, false, true);
        emit E(5);
        vm.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitNestedWindow();
    }

    /// emitWindow() emits events A, B, C, D, E.
    /// We should not be able to match [A, A] even if emitWindow() is called twice,
    /// as expectEmit() only works for the next call.
    function testShouldFailEventsOnTwoCalls() public {
        vm.expectEmit(true, false, false, true);
        emit A(1);
        vm.expectEmit(true, false, false, true);
        emit A(1);
        emitter.emitWindow();
        emitter.emitWindow();
    }

    /// We should not be able to expect emits if we're expecting the function reverts, no matter
    /// if the function reverts or not.
    function testShouldFailEmitWindowWithRevertDisallowed() public {
        vm.expectRevert();
        vm.expectEmit(true, false, false, true);
        emit A(1);
        emitter.emitWindow();
    }
}

contract ExpectEmitCountFailureTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Emitter emitter;

    event Something(uint256 indexed topic1, uint256 indexed topic2, uint256 indexed topic3, uint256 data);

    function setUp() public {
        emitter = new Emitter();
    }

    function testShouldFailNoEmit() public {
        vm.expectEmit(0);
        emit Something(1, 2, 3, 4);
        emitter.emitEvent(1, 2, 3, 4);
    }

    function testShouldFailCountLessEmits() public {
        uint64 count = 2;
        vm.expectEmit(count);
        emit Something(1, 2, 3, 4);
        emitter.emitNEvents(1, 2, 3, 4, count - 1);
    }

    function testShouldFailNoEmitFromAddress() public {
        vm.expectEmit(address(emitter), 0);
        emit Something(1, 2, 3, 4);
        emitter.emitEvent(1, 2, 3, 4);
    }

    function testShouldFailCountEmitsFromAddress() public {
        uint64 count = 3;
        vm.expectEmit(address(emitter), count);
        emit Something(1, 2, 3, 4);
        emitter.emitNEvents(1, 2, 3, 4, count - 1);
    }

    function testShouldFailEmitSomethingElse() public {
        uint64 count = 2;
        vm.expectEmit(count);
        emit Something(1, 2, 3, 4);
        emitter.emitSomethingElse(23214);
    }
}
