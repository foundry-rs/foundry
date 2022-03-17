// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Emitter {
    event Something(
        uint256 indexed topic1,
        uint256 indexed topic2,
        uint256 indexed topic3,
        uint256 data
    );

    function emitEvent(
        uint256 topic1,
        uint256 topic2,
        uint256 topic3,
        uint256 data
    ) public {
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

    function emitNested(
        Emitter inner,
        uint256 topic1,
        uint256 topic2,
        uint256 topic3,
        uint256 data
    ) public {
        inner.emitEvent(topic1, topic2, topic3, data);
    }
}

contract ExpectEmitTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    Emitter emitter;

    event Something(
        uint256 indexed topic1,
        uint256 indexed topic2,
        uint256 indexed topic3,
        uint256 data
    );

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
        cheats.expectEmit(true, true, true, true);
        emit Something(1, 2, 3, 4);
        cheats.expectEmit(true, true, true, true);
        emit Something(5, 6, 7, 8);

        emitter.emitMultiple(
            [uint256(1), uint256(5)],
            [uint256(2), uint256(6)],
            [uint256(3), uint256(7)],
            [uint256(4), uint256(8)]
        );
    }
}
