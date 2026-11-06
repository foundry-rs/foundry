// SPDX-License-Identifier: MIT
pragma solidity >=0.8.18;

interface RecordingVm {
    function record() external;
    function accesses(address target) external returns (bytes32[] memory reads, bytes32[] memory writes);
    function load(address target, bytes32 slot) external view returns (bytes32);
}

contract RecordingChild {
    RecordingVm constant vm = RecordingVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    constructor(bool recording) {
        if (recording) vm.record();
    }

    function start() external {
        vm.record();
    }
}

contract DynamicInspectorTest {
    RecordingVm constant vm = RecordingVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    RecordingChild child;

    error UnexpectedWriteCount(uint256 observed);

    function setUp() public {
        child = new RecordingChild(false);
    }

    function testDirectCheatcode() public {
        vm.record();
        observe();
    }

    function testCall() public {
        child.start();
        observe();
    }

    function testCallcode() public {
        address target = address(child);
        bytes memory input = abi.encodeCall(RecordingChild.start, ());
        bool success;
        assembly {
            success := callcode(gas(), target, 0, add(input, 32), mload(input), 0, 0)
        }
        require(success, "CALLCODE failed");
        observe();
    }

    function testDelegatecall() public {
        (bool success,) = address(child).delegatecall(abi.encodeCall(RecordingChild.start, ()));
        require(success, "DELEGATECALL failed");
        observe();
    }

    function testStaticcall() public {
        (bool success,) = address(child).staticcall(abi.encodeCall(RecordingChild.start, ()));
        require(success, "STATICCALL failed");
        observe();
    }

    function testCreate() public {
        new RecordingChild(true);
        observe();
    }

    function testCreate2() public {
        new RecordingChild{salt: bytes32(uint256(123))}(true);
        observe();
    }

    function testAlreadyRecording() public {
        vm.record();
        // Enter a fresh frame with recording already enabled: the stale-mask control.
        this.recordInFreshFrame();
    }

    function recordInFreshFrame() external {
        child.start();
        observe();
    }

    function observe() internal {
        // No intervening call may refresh the suspended caller's mask before this SSTORE.
        assembly {
            sstore(7, 11)
        }
        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(this));
        require(vm.load(address(this), bytes32(uint256(7))) == bytes32(uint256(11)), "store failed");
        if (writes.length != 1) revert UnexpectedWriteCount(writes.length);
        require(writes[0] == bytes32(uint256(7)), "wrong write slot");
        require(reads.length == 1 && reads[0] == writes[0], "wrong read slot");
    }
}
