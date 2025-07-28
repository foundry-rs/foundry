// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.18;

import "./test.sol";
import "./Vm.sol";

import {EventEmitter, SelectiveEmitter, ParamNumberingEmitter, AnonymousEmitter, ManyParamsEmitter} from "./ExpectEmitParamHarness.sol";

contract ExpectEmitParamFailures is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    // Contract instances
    EventEmitter public eventEmitter;
    SelectiveEmitter public selectiveEmitter;
    ParamNumberingEmitter public paramNumberingEmitter;
    AnonymousEmitter public anonymousEmitter;
    ManyParamsEmitter public manyParamsEmitter;

    // Event declarations for tests
    event SimpleEvent(uint256 indexed a, uint256 indexed b, uint256 c);
    event ComplexEvent(address indexed sender, uint256 indexed id, bytes data);
    event TestEvent(uint256 indexed a, uint256 indexed b, uint256 c);

    event MixedEventNumbering(
        uint256 indexed param0,
        address indexed param1,
        uint256 param2,
        uint256 param3,
        address param4
    );

    // Anonymous event for tests
    event AnonymousIndexed(uint256 indexed a, uint256 b, address c) anonymous;

    // Event with many parameters
    event ManyParams(uint256 a, uint256 b, uint256 c, uint256 d, uint256 e);

    function setUp() public {
        eventEmitter = new EventEmitter();
        selectiveEmitter = new SelectiveEmitter();
        paramNumberingEmitter = new ParamNumberingEmitter();
        anonymousEmitter = new AnonymousEmitter();
        manyParamsEmitter = new ManyParamsEmitter();
    }

    function testIndexedParamMismatch() public {
        vm.expectEmit(true, true, true, true);
        emit SimpleEvent(100, 200, 300);
        eventEmitter.emitSimple(100, 999, 300); // Second indexed param (b) mismatch
    }

    function testNonIndexedParamMismatch() public {
        vm.expectEmit(true, true, true, true);
        emit SimpleEvent(100, 200, 300);
        eventEmitter.emitSimple(100, 200, 999); // Non-indexed param (c) mismatch
    }

    function testMultipleMismatches() public {
        vm.expectEmit(true, true, true, true);
        emit SimpleEvent(100, 200, 300);
        eventEmitter.emitSimple(999, 888, 777); // All params mismatch
    }

    function testSelectiveChecks() public {
        vm.expectEmit(true, false, true, true); // checkTopic2=false
        emit TestEvent(100, 200, 300);
        selectiveEmitter.emitEvent(100, 999, 300); // Topic2 different but not checked
    }

    function testParameterNumbering() public {
        vm.expectEmit(true, true, true, true);
        emit MixedEventNumbering(
            100,
            address(0x1234),
            300,
            400,
            address(0x5678)
        );
        paramNumberingEmitter.emitEvent(
            100,
            address(0x1234),
            999,
            400,
            address(0x5678)
        ); // param2 mismatch
    }

    function testCompletelyDifferentEvent() public {
        vm.expectEmit(true, true, true, true);
        emit SimpleEvent(100, 200, 300);
        eventEmitter.emitComplex(address(this), 42, hex"deadbeef"); // Different event type
    }

    function testAnonymousEventMismatch() public {
        vm.expectEmitAnonymous(true, false, false, false, true); // Check topic0 and data
        emit AnonymousIndexed(100, 200, address(0x1234));
        anonymousEmitter.emitAnonymousIndexed(999, 200, address(0x1234)); // param0 mismatch
    }

    function testManyParameterMismatches() public {
        vm.expectEmit(true, true, true, true);
        // Event with 5 non-indexed parameters
        emit ManyParams(100, 200, 300, 400, 500);
        // All 5 parameters differ - should show each one individually
        manyParamsEmitter.emitManyParams(111, 222, 333, 444, 555);
    }

    function testMixedEventNonIndexedMismatch() public {
        // For SimpleEvent: 'a' and 'b' are indexed, 'c' is non-indexed
        vm.expectEmit(true, true, true, true);
        emit SimpleEvent(100, 200, 300);
        // Same indexed params (100, 200) but different non-indexed param
        eventEmitter.emitSimple(100, 200, 999);
    }
}
