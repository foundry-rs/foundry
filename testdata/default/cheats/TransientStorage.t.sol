// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.24;

import "utils/Test.sol";

interface TransientStorageCallback {
    function onTransientCallback() external;
}

contract TransientStorageTarget {
    bytes32 public constant LOCK_SLOT = keccak256("transient.lock");

    error Locked();

    uint256 public count;

    function guarded() external {
        if (locked()) revert Locked();
        count++;
    }

    function lockAndCallback() external {
        bytes32 slot = LOCK_SLOT;
        assembly {
            tstore(slot, 1)
        }
        TransientStorageCallback(msg.sender).onTransientCallback();
        assembly {
            tstore(slot, 0)
        }
    }

    function tstoreSlot(bytes32 slot, bytes32 value) external {
        assembly {
            tstore(slot, value)
        }
    }

    function locked() public view returns (bool isLocked) {
        bytes32 slot = LOCK_SLOT;
        assembly {
            isLocked := tload(slot)
        }
    }
}

contract TransientStorageCaller {
    Vm constant vm = Vm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));

    function storeAndCall(TransientStorageTarget target, bytes32 value) external returns (bool success) {
        vm.storeTransient(address(target), target.LOCK_SLOT(), value);
        (success,) = address(target).call(abi.encodeCall(TransientStorageTarget.guarded, ()));
    }

    function storeAndRevert(address target, bytes32 slot, bytes32 value) external {
        vm.storeTransient(target, slot, value);
        revert("reverted");
    }
}

abstract contract TransientStorageTestBase is Test, TransientStorageCallback {
    TransientStorageTarget target;
    TransientStorageCaller caller;
    bytes32 slot;
    bytes32 observed;

    function setUp() public {
        target = new TransientStorageTarget();
        caller = new TransientStorageCaller();
        slot = target.LOCK_SLOT();
    }

    function onTransientCallback() external {
        observed = vm.loadTransient(address(target), slot);
    }

    function testStoreTransientRoundTrip() public {
        assertEq(vm.loadTransient(address(target), slot), bytes32(0), "slot should start empty");

        vm.storeTransient(address(target), slot, bytes32(uint256(1)));
        assertEq(vm.loadTransient(address(target), slot), bytes32(uint256(1)), "loadTransient failed");

        vm.storeTransient(address(target), slot, bytes32(0));
        assertEq(vm.loadTransient(address(target), slot), bytes32(0), "slot was not cleared");
    }

    function testStoreTransientReachesCallInSameTransaction() public {
        assertTrue(!caller.storeAndCall(target, bytes32(uint256(1))), "target did not see the lock");
        assertTrue(caller.storeAndCall(target, bytes32(0)), "target saw a cleared lock");
    }

    function testLoadTransientDuringCall() public {
        target.lockAndCallback();
        assertEq(observed, bytes32(uint256(1)), "loadTransient did not see the target's tstore");
    }

    function testLoadTransientOwnStorage() public {
        bytes32 ownSlot = keccak256("own");
        assembly {
            tstore(ownSlot, 7)
        }
        assertEq(vm.loadTransient(address(this), ownSlot), bytes32(uint256(7)));
    }

    function testTransientAndPersistentStorageAreSeparate() public {
        vm.storeTransient(address(target), slot, bytes32(uint256(1)));
        assertEq(vm.load(address(target), slot), bytes32(0), "storeTransient wrote persistent storage");

        vm.store(address(target), slot, bytes32(uint256(2)));
        assertEq(vm.loadTransient(address(target), slot), bytes32(uint256(1)), "store wrote transient storage");
    }

    function testStoreTransientRevertedWithFrame() public {
        try caller.storeAndRevert(address(target), slot, bytes32(uint256(1))) {
            fail();
        } catch {}
        assertEq(vm.loadTransient(address(target), slot), bytes32(0), "reverted write was kept");
    }

    function testStoreTransientNotAvailableOnPrecompiles() public {
        vm._expectCheatcodeRevert("cannot use precompile 0x0000000000000000000000000000000000000001 as an argument");
        vm.storeTransient(address(1), slot, bytes32(uint256(1)));
    }

    function testLoadTransientAvailableOnPrecompiles() public {
        assertEq(vm.loadTransient(address(1), slot), bytes32(0));
    }

    function testStoreTransientFuzzed(bytes32 fuzzSlot, bytes32 value) public {
        vm.storeTransient(address(target), fuzzSlot, value);
        assertEq(vm.loadTransient(address(target), fuzzSlot), value);
    }
}

/// Without isolation, the test and every call it makes share one transaction.
/// forge-config: default.isolate = false
contract TransientStorageTest is TransientStorageTestBase {
    function testStoreTransientReachesNextCall() public {
        vm.storeTransient(address(target), slot, bytes32(uint256(1)));
        vm.expectRevert(TransientStorageTarget.Locked.selector);
        target.guarded();
    }

    function testLoadTransientAfterCall() public {
        target.tstoreSlot(slot, bytes32(uint256(42)));
        assertEq(vm.loadTransient(address(target), slot), bytes32(uint256(42)));
    }
}

/// With isolation, each top-level call runs as its own transaction and starts with empty transient
/// storage, and its transient writes end with it.
/// forge-config: default.isolate = true
contract TransientStorageIsolatedTest is TransientStorageTestBase {
    function testIsolatedCallStartsWithEmptyTransientStorage() public {
        assertTrue(vm.isIsolateMode());
        vm.storeTransient(address(target), slot, bytes32(uint256(1)));

        target.guarded();
        assertEq(target.count(), 1);
        assertEq(vm.loadTransient(address(target), slot), bytes32(uint256(1)));
    }

    function testIsolatedCallTransientWritesEndWithCall() public {
        target.tstoreSlot(slot, bytes32(uint256(42)));
        assertEq(vm.loadTransient(address(target), slot), bytes32(0));
    }
}

/// `setUp` and each test run as separate transactions, so transient storage does not carry over.
contract TransientStorageSetUpTest is Test {
    TransientStorageTarget target;
    bytes32 slot;

    function setUp() public {
        target = new TransientStorageTarget();
        slot = target.LOCK_SLOT();
        vm.storeTransient(address(target), slot, bytes32(uint256(1)));
        assertEq(vm.loadTransient(address(target), slot), bytes32(uint256(1)));
    }

    function testTransientStorageClearedAfterSetUp() public {
        assertEq(vm.loadTransient(address(target), slot), bytes32(0), "value leaked from setUp");
        target.guarded();
        assertEq(target.count(), 1);
    }
}
