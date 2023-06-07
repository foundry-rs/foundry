// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

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

    function getVar() public view returns (uint256) {
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

contract ExpectEmitTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    Emitter emitter;

    event Something(uint256 indexed topic1, uint256 indexed topic2, uint256 indexed topic3, uint256 data);

    event SomethingElse(uint256 indexed topic1);

    event SomethingNonIndexed(uint256 data);

    event A(uint256 indexed topic1);
    event B(uint256 indexed topic1);
    event C(uint256 indexed topic1);
    event D(uint256 indexed topic1);
    event E(uint256 indexed topic1);

    function setUp() public {
        emitter = new Emitter();
    }

    function testFailExpectEmitDanglingNoReference() public {
        cheats.expectEmit(false, false, false, false);
    }

    function testFailExpectEmitDanglingWithReference() public {
        cheats.expectEmit(false, false, false, false);
        emit Something(1, 2, 3, 4);
    }

    /// The topics that are not checked are altered to be incorrect
    /// compared to the reference.
    function testExpectEmit(
        bool checkTopic1,
        bool checkTopic2,
        bool checkTopic3,
        bool checkData,
        uint128 topic1,
        uint128 topic2,
        uint128 topic3,
        uint128 data
    ) public {
        uint256 transformedTopic1 = checkTopic1 ? uint256(topic1) : uint256(topic1) + 1;
        uint256 transformedTopic2 = checkTopic2 ? uint256(topic2) : uint256(topic2) + 1;
        uint256 transformedTopic3 = checkTopic3 ? uint256(topic3) : uint256(topic3) + 1;
        uint256 transformedData = checkData ? uint256(data) : uint256(data) + 1;

        cheats.expectEmit(checkTopic1, checkTopic2, checkTopic3, checkData);

        emit Something(topic1, topic2, topic3, data);
        emitter.emitEvent(transformedTopic1, transformedTopic2, transformedTopic3, transformedData);
    }

    /// The topics that are checked are altered to be incorrect
    /// compared to the reference.
    function testFailExpectEmit(
        bool checkTopic1,
        bool checkTopic2,
        bool checkTopic3,
        bool checkData,
        uint128 topic1,
        uint128 topic2,
        uint128 topic3,
        uint128 data
    ) public {
        cheats.assume(checkTopic1 || checkTopic2 || checkTopic3 || checkData);

        uint256 transformedTopic1 = checkTopic1 ? uint256(topic1) + 1 : uint256(topic1);
        uint256 transformedTopic2 = checkTopic2 ? uint256(topic2) + 1 : uint256(topic2);
        uint256 transformedTopic3 = checkTopic3 ? uint256(topic3) + 1 : uint256(topic3);
        uint256 transformedData = checkData ? uint256(data) + 1 : uint256(data);

        cheats.expectEmit(checkTopic1, checkTopic2, checkTopic3, checkData);

        emit Something(topic1, topic2, topic3, data);
        emitter.emitEvent(transformedTopic1, transformedTopic2, transformedTopic3, transformedData);
    }

    /// The topics that are checked are altered to be incorrect
    /// compared to the reference.
    function testExpectEmitNested(
        bool checkTopic1,
        bool checkTopic2,
        bool checkTopic3,
        bool checkData,
        uint128 topic1,
        uint128 topic2,
        uint128 topic3,
        uint128 data
    ) public {
        Emitter inner = new Emitter();

        uint256 transformedTopic1 = checkTopic1 ? uint256(topic1) : uint256(topic1) + 1;
        uint256 transformedTopic2 = checkTopic2 ? uint256(topic2) : uint256(topic2) + 1;
        uint256 transformedTopic3 = checkTopic3 ? uint256(topic3) : uint256(topic3) + 1;
        uint256 transformedData = checkData ? uint256(data) : uint256(data) + 1;

        cheats.expectEmit(checkTopic1, checkTopic2, checkTopic3, checkData);

        emit Something(topic1, topic2, topic3, data);
        emitter.emitNested(inner, transformedTopic1, transformedTopic2, transformedTopic3, transformedData);
    }

    /// The topics that are checked are altered to be incorrect
    /// compared to the reference.
    function testFailExpectEmitNested(
        bool checkTopic1,
        bool checkTopic2,
        bool checkTopic3,
        bool checkData,
        uint128 topic1,
        uint128 topic2,
        uint128 topic3,
        uint128 data
    ) public {
        cheats.assume(checkTopic1 || checkTopic2 || checkTopic3 || checkData);
        Emitter inner = new Emitter();

        uint256 transformedTopic1 = checkTopic1 ? uint256(topic1) + 1 : uint256(topic1);
        uint256 transformedTopic2 = checkTopic2 ? uint256(topic2) + 1 : uint256(topic2);
        uint256 transformedTopic3 = checkTopic3 ? uint256(topic3) + 1 : uint256(topic3);
        uint256 transformedData = checkData ? uint256(data) + 1 : uint256(data);

        cheats.expectEmit(checkTopic1, checkTopic2, checkTopic3, checkData);

        emit Something(topic1, topic2, topic3, data);
        emitter.emitNested(inner, transformedTopic1, transformedTopic2, transformedTopic3, transformedData);
    }

    function testExpectEmitMultiple() public {
        cheats.expectEmit();
        emit Something(1, 2, 3, 4);
        cheats.expectEmit();
        emit Something(5, 6, 7, 8);

        emitter.emitMultiple(
            [uint256(1), uint256(5)], [uint256(2), uint256(6)], [uint256(3), uint256(7)], [uint256(4), uint256(8)]
        );
    }

    function testExpectedEmitMultipleNested() public {
        cheats.expectEmit();
        emit Something(1, 2, 3, 4);
        cheats.expectEmit();
        emit Something(1, 2, 3, 4);

        emitter.emitAndNest();
    }

    function testExpectEmitMultipleWithArgs() public {
        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);
        cheats.expectEmit(true, true, true, true);
        emit Something(5, 6, 7, 8);

        emitter.emitMultiple(
            [uint256(1), uint256(5)], [uint256(2), uint256(6)], [uint256(3), uint256(7)], [uint256(4), uint256(8)]
        );
    }

    function testExpectEmitCanMatchWithoutExactOrder() public {
        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);
        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);

        emitter.emitOutOfExactOrder();
    }

    function testFailExpectEmitCanMatchWithoutExactOrder() public {
        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);
        // This should fail, as this event is never emitted
        // in between the other two Something events.
        cheats.expectEmit(true, true, true, true);
        emit SomethingElse(1);
        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);

        emitter.emitOutOfExactOrder();
    }

    function testExpectEmitCanMatchWithoutExactOrder2() public {
        cheats.expectEmit(true, true, true, true);
        emit SomethingNonIndexed(1);
        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);

        emitter.emitOutOfExactOrder();
    }

    function testExpectEmitAddress() public {
        cheats.expectEmit(address(emitter));
        emit Something(1, 2, 3, 4);

        emitter.emitEvent(1, 2, 3, 4);
    }

    function testExpectEmitAddressWithArgs() public {
        cheats.expectEmit(true, true, true, true, address(emitter));
        emit Something(1, 2, 3, 4);

        emitter.emitEvent(1, 2, 3, 4);
    }

    function testFailExpectEmitAddress() public {
        cheats.expectEmit(address(0));
        emit Something(1, 2, 3, 4);

        emitter.emitEvent(1, 2, 3, 4);
    }

    function testFailExpectEmitAddressWithArgs() public {
        cheats.expectEmit(true, true, true, true, address(0));
        emit Something(1, 2, 3, 4);

        emitter.emitEvent(1, 2, 3, 4);
    }

    /// Ref: issue #760
    function testFailLowLevelWithoutEmit() public {
        LowLevelCaller caller = new LowLevelCaller();

        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);

        // This does not emit an event, so this test should fail
        caller.f();
    }

    function testFailNoEmitDirectlyOnNextCall() public {
        LowLevelCaller caller = new LowLevelCaller();

        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);

        // This call does not emit. As emit expects the next call to emit, this should fail.
        caller.f();
        // This call does emit, but it is a call later than expected.
        emitter.emitEvent(1, 2, 3, 4);
    }

    /// Ref: issue #760
    function testFailDifferentIndexedParameters() public {
        cheats.expectEmit(true, false, false, false);
        emit SomethingElse(1);

        // This should fail since `SomethingElse` in the test
        // and in the `Emitter` contract have differing
        // amounts of indexed topics.
        emitter.emitSomethingElse(1);
    }

    function testCanDoStaticCall() public {
        cheats.expectEmit(true, true, true, true);
        emit Something(emitter.getVar(), 2, 3, 4);

        emitter.emitEvent(1, 2, 3, 4);
    }

    /// Tests for additive behavior.
    // As long as we match the event we want in order, it doesn't matter which events are emitted afterwards.
    function testAdditiveBehavior() public {
        cheats.expectEmit(true, true, true, true, address(emitter));
        emit Something(1, 2, 3, 4);

        emitter.emitMultiple(
            [uint256(1), uint256(5)], [uint256(2), uint256(6)], [uint256(3), uint256(7)], [uint256(4), uint256(8)]
        );
    }

    /// This test should fail, as the call to `changeThing` is not a static call.
    /// While we can ignore static calls, we cannot ignore normal calls.
    function testFailEmitOnlyAppliesToNextCall() public {
        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);
        // This works because it's a staticcall.
        emitter.doesNothing();
        // This should make the test fail as it's a normal call.
        emitter.changeThing(block.timestamp);

        emitter.emitEvent(1, 2, 3, 4);
    }

    /// emitWindow() emits events A, B, C, D, E.
    /// We should be able to match [A, B, C, D, E] in the correct order.
    function testCanMatchConsecutiveEvents() public {
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit B(2);
        cheats.expectEmit(true, false, false, true);
        emit C(3);
        cheats.expectEmit(true, false, false, true);
        emit D(4);
        cheats.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitWindow();
    }

    /// emitWindow() emits events A, B, C, D, E.
    /// We should be able to match [A, C, E], as they're in the right order,
    /// even if they're not consecutive.
    function testCanMatchConsecutiveEventsSkipped() public {
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit C(3);
        cheats.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitWindow();
    }

    /// emitWindow() emits events A, B, C, D, E.
    /// We should be able to match [C, E], as they're in the right order,
    /// even if they're not consecutive.
    function testCanMatchConsecutiveEventsSkipped2() public {
        cheats.expectEmit(true, false, false, true);
        emit C(3);
        cheats.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitWindow();
    }

    /// emitWindow() emits events A, B, C, D, E.
    /// We should be able to match [C], as it's contained in the events emitted,
    /// even if we don't match the previous or following ones.
    function testCanMatchSingleEventFromConsecutive() public {
        cheats.expectEmit(true, false, false, true);
        emit C(3);

        emitter.emitWindow();
    }

    /// emitWindow() emits events A, B, C, D, E.
    /// We should not be able to match [B, A, C, D, E] as B and A are flipped.
    function testFailCanMatchConsecutiveEvents() public {
        cheats.expectEmit(true, false, false, true);
        emit B(2);
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit C(3);
        cheats.expectEmit(true, false, false, true);
        emit D(4);
        cheats.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitWindow();
    }

    /// emitWindowNested() emits events A, C, E, A, B, C, D, E, the last 5 on an external call.
    /// We should be able to match the whole event sequence in order no matter if the events
    /// were emitted deeper into the call tree.
    function testCanMatchConsecutiveNestedEvents() public {
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit C(3);
        cheats.expectEmit(true, false, false, true);
        emit E(5);
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit B(2);
        cheats.expectEmit(true, false, false, true);
        emit C(3);
        cheats.expectEmit(true, false, false, true);
        emit D(4);
        cheats.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitNestedWindow();
    }

    /// emitWindowNested() emits events A, C, E, A, B, C, D, E, the last 5 on an external call.
    /// We should be able to match [A, C, E, A, C, E] in that order, as these are emitted twice.
    function testCanMatchRepeatedEvents() public {
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit C(3);
        cheats.expectEmit(true, false, false, true);
        emit E(5);
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit C(3);
        cheats.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitNestedWindow();
    }

    /// emitWindowNested() emits events A, C, E, A, B, C, D, E, the last 5 on an external call.
    /// We should NOT be able to match [A, A, E, E], as while we're matching the correct amount
    /// of events, they're not in the correct order. It should be [A, E, A, E].
    function testFailMatchRepeatedEventsOutOfOrder() public {
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit E(5);
        cheats.expectEmit(true, false, false, true);
        emit E(5);

        emitter.emitNestedWindow();
    }

    /// emitWindow() emits events A, B, C, D, E.
    /// We should not be able to match [A, A] even if emitWindow() is called twice,
    /// as expectEmit() only works for the next call.
    function testFailEventsOnTwoCalls() public {
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        cheats.expectEmit(true, false, false, true);
        emit A(1);
        emitter.emitWindow();
        emitter.emitWindow();
    }

    /// This test will fail if we check that all expected logs were emitted
    /// after every call from the same depth as the call that invoked the cheatcode.
    ///
    /// Expected emits should only be checked when the call from which the cheatcode
    /// was invoked ends.
    ///
    /// Ref: issue #1214
    /// NOTE: This is now invalid behavior.
    // function testExpectEmitIsCheckedWhenCurrentCallTerminates() public {
    //     cheats.expectEmit(true, true, true, true);
    //     emitter.doesNothing();
    //     emit Something(1, 2, 3, 4);

    //     // This should fail since `SomethingElse` in the test
    //     // and in the `Emitter` contract have differing
    //     // amounts of indexed topics.
    //     emitter.emitEvent(1, 2, 3, 4);
    // }
}
