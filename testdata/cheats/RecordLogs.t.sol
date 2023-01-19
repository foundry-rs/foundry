// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Emitter {
    event LogAnonymous(bytes data) anonymous;

    event LogTopic0(bytes data);

    event LogTopic1(uint256 indexed topic1, bytes data);

    event LogTopic12(uint256 indexed topic1, uint256 indexed topic2, bytes data);

    event LogTopic123(uint256 indexed topic1, uint256 indexed topic2, uint256 indexed topic3, bytes data);

    function emitAnonymousEvent(bytes memory data) public {
        emit LogAnonymous(data);
    }

    function emitEvent(bytes memory data) public {
        emit LogTopic0(data);
    }

    function emitEvent(uint256 topic1, bytes memory data) public {
        emit LogTopic1(topic1, data);
    }

    function emitEvent(uint256 topic1, uint256 topic2, bytes memory data) public {
        emit LogTopic12(topic1, topic2, data);
    }

    function emitEvent(uint256 topic1, uint256 topic2, uint256 topic3, bytes memory data) public {
        emit LogTopic123(topic1, topic2, topic3, data);
    }
}

contract Emitterv2 {
    Emitter emitter = new Emitter();

    function emitEvent(uint256 topic1, uint256 topic2, uint256 topic3, bytes memory data) public {
        emitter.emitEvent(topic1, topic2, topic3, data);
    }

    function getEmitterAddr() public view returns (address) {
        return address(emitter);
    }
}

contract RecordLogsTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    Emitter emitter;
    bytes32 internal seedTestData = keccak256(abi.encodePacked("Some data"));

    // Used on testRecordOnEmitDifferentDepths()
    event LogTopic(uint256 indexed topic1, bytes data);

    function setUp() public {
        emitter = new Emitter();
    }

    function generateTestData(uint8 n) internal returns (bytes memory) {
        bytes memory output = new bytes(n);

        for (uint8 i = 0; i < n; i++) {
            output[i] = seedTestData[i % 32];
            if (i % 32 == 31) {
                seedTestData = keccak256(abi.encodePacked(seedTestData));
            }
        }

        return output;
    }

    function testRecordOffGetsNothing() public {
        emitter.emitEvent(1, 2, 3, generateTestData(48));
        Cheats.Log[] memory entries = cheats.getRecordedLogs();

        assertEq(entries.length, 0);
    }

    function testRecordOnNoLogs() public {
        cheats.recordLogs();
        Cheats.Log[] memory entries = cheats.getRecordedLogs();

        assertEq(entries.length, 0);
    }

    function testRecordOnSingleLog() public {
        bytes memory testData = "Event Data in String";

        cheats.recordLogs();
        emitter.emitEvent(1, 2, 3, testData);
        Cheats.Log[] memory entries = cheats.getRecordedLogs();

        assertEq(entries.length, 1);
        assertEq(entries[0].topics.length, 4);
        assertEq(entries[0].topics[0], keccak256("LogTopic123(uint256,uint256,uint256,bytes)"));
        assertEq(entries[0].topics[1], bytes32(uint256(1)));
        assertEq(entries[0].topics[2], bytes32(uint256(2)));
        assertEq(entries[0].topics[3], bytes32(uint256(3)));
        assertEq(abi.decode(entries[0].data, (string)), string(testData));
        assertEq(entries[0].emitter, address(emitter));
    }

    // TODO
    // This crashes on decoding!
    //   The application panicked (crashed).
    //   Message:  index out of bounds: the len is 0 but the index is 0
    //   Location: <local-dir>/evm/src/trace/decoder.rs:299
    function NOtestRecordOnAnonymousEvent() public {
        bytes memory testData = generateTestData(48);

        cheats.recordLogs();
        emitter.emitAnonymousEvent(testData);
        Cheats.Log[] memory entries = cheats.getRecordedLogs();

        assertEq(entries.length, 1);
    }

    function testRecordOnSingleLogTopic0() public {
        bytes memory testData = generateTestData(48);

        cheats.recordLogs();
        emitter.emitEvent(testData);
        Cheats.Log[] memory entries = cheats.getRecordedLogs();

        assertEq(entries.length, 1);
        assertEq(entries[0].topics.length, 1);
        assertEq(entries[0].topics[0], keccak256("LogTopic0(bytes)"));
        // While not a proper string, this conversion allows the comparison.
        assertEq(abi.decode(entries[0].data, (string)), string(testData));
        assertEq(entries[0].emitter, address(emitter));
    }

    function testEmitRecordEmit() public {
        bytes memory testData0 = generateTestData(32);
        emitter.emitEvent(1, 2, testData0);

        cheats.recordLogs();
        bytes memory testData1 = generateTestData(16);
        emitter.emitEvent(3, testData1);
        Cheats.Log[] memory entries = cheats.getRecordedLogs();

        assertEq(entries.length, 1);
        assertEq(entries[0].topics.length, 2);
        assertEq(entries[0].topics[0], keccak256("LogTopic1(uint256,bytes)"));
        assertEq(entries[0].topics[1], bytes32(uint256(3)));
        assertEq(abi.decode(entries[0].data, (string)), string(testData1));
        assertEq(entries[0].emitter, address(emitter));
    }

    function testRecordOnEmitDifferentDepths() public {
        cheats.recordLogs();

        bytes memory testData0 = generateTestData(16);
        emit LogTopic(1, testData0);

        bytes memory testData1 = generateTestData(20);
        emitter.emitEvent(2, 3, testData1);

        bytes memory testData2 = generateTestData(24);
        Emitterv2 emitter2 = new Emitterv2();
        emitter2.emitEvent(4, 5, 6, testData2);

        Cheats.Log[] memory entries = cheats.getRecordedLogs();

        assertEq(entries.length, 3);

        assertEq(entries[0].topics.length, 2);
        assertEq(entries[0].topics[0], keccak256("LogTopic(uint256,bytes)"));
        assertEq(entries[0].topics[1], bytes32(uint256(1)));
        assertEq(abi.decode(entries[0].data, (string)), string(testData0));
        assertEq(entries[0].emitter, address(this));

        assertEq(entries[1].topics.length, 3);
        assertEq(entries[1].topics[0], keccak256("LogTopic12(uint256,uint256,bytes)"));
        assertEq(entries[1].topics[1], bytes32(uint256(2)));
        assertEq(entries[1].topics[2], bytes32(uint256(3)));
        assertEq(abi.decode(entries[1].data, (string)), string(testData1));
        assertEq(entries[1].emitter, address(emitter));

        assertEq(entries[2].topics.length, 4);
        assertEq(entries[2].topics[0], keccak256("LogTopic123(uint256,uint256,uint256,bytes)"));
        assertEq(entries[2].topics[1], bytes32(uint256(4)));
        assertEq(entries[2].topics[2], bytes32(uint256(5)));
        assertEq(entries[2].topics[3], bytes32(uint256(6)));
        assertEq(abi.decode(entries[2].data, (string)), string(testData2));
        assertEq(entries[2].emitter, emitter2.getEmitterAddr());
    }

    function testRecordsConsumednAsRead() public {
        Cheats.Log[] memory entries;

        emitter.emitEvent(1, generateTestData(16));

        // hit record now
        cheats.recordLogs();

        entries = cheats.getRecordedLogs();
        assertEq(entries.length, 0);

        // emit after calling .getRecordedLogs()
        emitter.emitEvent(2, 3, generateTestData(24));

        entries = cheats.getRecordedLogs();
        assertEq(entries.length, 1);
        assertEq(entries[0].topics.length, 3);
        assertEq(entries[0].emitter, address(emitter));

        // let's emit two more!
        emitter.emitEvent(4, 5, 6, generateTestData(20));
        emitter.emitEvent(generateTestData(32));

        entries = cheats.getRecordedLogs();
        assertEq(entries.length, 2);
        assertEq(entries[0].topics.length, 4);
        assertEq(entries[1].topics.length, 1);
        assertEq(entries[0].emitter, address(emitter));
        assertEq(entries[1].emitter, address(emitter));

        // the last one
        emitter.emitEvent(7, 8, 9, generateTestData(24));

        entries = cheats.getRecordedLogs();
        assertEq(entries.length, 1);
        assertEq(entries[0].topics.length, 4);
        assertEq(entries[0].emitter, address(emitter));
    }
}
