// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract MappingHookTarget {
    uint256 public scalar;
    mapping(address => uint256) public balances;
    mapping(address => mapping(address => uint256)) public allowances;

    function setBalance(address owner, uint256 value) external {
        balances[owner] = value;
    }

    function setAllowance(address owner, address spender, uint256 value) external {
        allowances[owner][spender] = value;
    }

    function compute(bytes32 key, bytes32 root) external pure returns (bytes32) {
        return keccak256(abi.encode(key, root));
    }

    function directStore(bytes32 slot, uint256 value) external {
        assembly { sstore(slot, value) }
    }

    function offsetStore(address owner, uint256 value) external {
        bytes32 slot = bytes32(uint256(keccak256(abi.encode(owner, uint256(1)))) + 1);
        assembly { sstore(slot, value) }
    }

    function incompleteStore(bytes32 key, uint256 value) external {
        bytes32 slot = keccak256(abi.encodePacked(key));
        assembly { sstore(slot, value) }
    }

    function unallocatedMemoryStore(uint256 value) external {
        assembly {
            let slot := keccak256(0x1000, 0x40)
            sstore(slot, value)
        }
    }

    function failedHashThenStore(address implementation, bytes32 slot, uint256 value) external {
        (bool success,) = implementation.delegatecall{gas: 5_000}("");
        require(!success);
        assembly { sstore(slot, value) }
    }
}

contract MappingHookImplementation {
    function setBalance(address owner, uint256 value) external {
        bytes32 slot = keccak256(abi.encode(owner, uint256(1)));
        assembly { sstore(slot, value) }
    }
}

contract MappingHookProxy {
    function setBalance(address implementation, address owner, uint256 value) external {
        (bool ok,) = implementation.delegatecall(abi.encodeCall(MappingHookImplementation.setBalance, (owner, value)));
        require(ok);
    }
}

contract MappingStorageHooksTest is Test {
    MappingHookTarget target;
    uint256 calls;
    uint256 implementationCalls;
    uint256 intermediateCalls;
    mapping(uint256 => uint256) ghost;
    bytes32[] keys;
    bytes32 staleSlot;
    bytes32 seenSlot;
    bytes32 seenOld;
    bytes32 seenNew;

    modifier onlyStorageHook() {
        require(msg.sender == address(vm), "only storage hook");
        _;
    }

    function setUp() public {
        target = new MappingHookTarget();
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(1)), this.onBalance.selector);
        staleSlot = target.compute(bytes32(uint256(4)), bytes32(uint256(1)));
    }

    function testArgumentsNestedOrderAndRootFiltering() public {
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(1)), this.onBalance.selector);
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(2)), this.onAllowance.selector);
        target.setBalance(address(0xA11CE), 3);
        assertEq(calls, 1);
        assertEq(seenSlot, keccak256(abi.encode(address(0xA11CE), uint256(1))));
        assertEq(seenOld, bytes32(0));
        assertEq(seenNew, bytes32(uint256(3)));
        target.setAllowance(address(0xA11CE), address(0xB0B), 7);
        assertEq(calls, 2);
        assertEq(keys.length, 2);
        assertEq(keys[0], bytes32(uint256(uint160(address(0xA11CE)))));
        assertEq(keys[1], bytes32(uint256(uint160(address(0xB0B)))));
        target.scalar();
        assertEq(calls, 2);
    }

    function testOffsetAndIncompleteStoresDoNotMatch() public {
        target.offsetStore(address(this), 1);
        target.incompleteStore(bytes32(uint256(4)), 1);
        assertEq(calls, 0);
    }

    function testUnallocatedKeccakMemoryIsZeroExpanded() public {
        vm.registerMappingSstoreHook(address(target), bytes32(0), this.onZeroMemory.selector);
        target.unallocatedMemoryStore(1);
        assertEq(calls, 1);
        assertEq(keys.length, 1);
        assertEq(keys[0], bytes32(0));
        assertEq(seenSlot, keccak256(new bytes(64)));
    }

    function testOogKeccakDoesNotRecordProvenance() public {
        address implementation = address(0xBEEF);
        vm.etch(implementation, hex"604062100000205000");
        vm.registerMappingSstoreHook(address(target), bytes32(0), this.onZeroMemory.selector);
        target.failedHashThenStore(implementation, keccak256(new bytes(64)), 1);
        assertEq(calls, 0);
    }

    function testPriorTopLevelProvenanceDoesNotMatch() public {
        target.directStore(staleSlot, 1);
        assertEq(calls, 0);
    }

    function testLateRootRegistrationClearsProvenance() public {
        bytes32 root = bytes32(uint256(72));
        bytes32 slot = target.compute(bytes32(uint256(4)), root);
        vm.registerMappingSstoreHook(address(target), root, this.onIntermediate.selector);
        target.directStore(slot, 1);
        assertEq(intermediateCalls, 0);
    }

    function testResolutionUsesTerminalRoot() public {
        address owner = address(0xA11CE);
        address spender = address(0xB0B);
        bytes32 intermediateRoot = target.compute(bytes32(uint256(uint160(owner))), bytes32(uint256(2)));
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(2)), this.onAllowance.selector);
        vm.registerMappingSstoreHook(address(target), intermediateRoot, this.onIntermediate.selector);

        target.setAllowance(owner, spender, 7);

        assertEq(calls, 1);
        assertEq(intermediateCalls, 0);
        assertEq(keys.length, 2);
    }

    function testReplacementAndRawConflict() public {
        bool success;
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(1)), this.onBalance.selector);
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(1)), this.onReplacement.selector);
        target.setBalance(address(this), 1);
        assertEq(calls, 10);
        (success,) = address(vm).call(abi.encodeCall(vm.registerSstoreHook, (address(target), this.onRaw.selector)));
        vm.assertFalse(success);
        MappingHookTarget raw = new MappingHookTarget();
        vm.registerSstoreHook(address(raw), this.onRaw.selector);
        (success,) = address(vm)
            .call(
                abi.encodeCall(
                    vm.registerMappingSstoreHook, (address(raw), bytes32(uint256(1)), this.onBalance.selector)
                )
            );
        vm.assertFalse(success);
    }

    function testEnclosingRevertRollsBackCallbackState() public {
        bool success;
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(1)), this.onBalance.selector);
        (success,) = address(this).call(abi.encodeCall(this.storeThenRevert, (address(0xBEEF), 9)));
        vm.assertFalse(success);
        assertEq(calls, 0);
        assertEq(target.balances(address(0xBEEF)), 0);
    }

    function testDelegatecallUsesProxyStorageAccount() public {
        MappingHookImplementation implementation = new MappingHookImplementation();
        MappingHookProxy proxy = new MappingHookProxy();
        vm.registerMappingSstoreHook(address(proxy), bytes32(uint256(1)), this.onBalance.selector);
        vm.registerMappingSstoreHook(address(implementation), bytes32(uint256(1)), this.onImplementation.selector);
        target = MappingHookTarget(address(proxy));

        proxy.setBalance(address(implementation), address(0xA11CE), 4);
        assertEq(calls, 1);
        assertEq(implementationCalls, 0);
    }

    function testCallbackSubtreeIsSuppressed() public {
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(1)), this.onRecursive.selector);
        target.setBalance(address(0xA11CE), 3);
        assertEq(calls, 1);
    }

    function testCallbackWritesAreExcludedFromMappingRecording() public {
        vm.startMappingRecording();
        target.setBalance(address(0xA11CE), 3);

        assertEq(vm.getMappingLength(address(this), ghostRoot()), 0);
        assertEq(vm.getMappingLength(address(target), bytes32(uint256(1))), 1);
    }

    function testCallbackRevertPropagates() public {
        bool success;
        bytes memory data;
        vm.registerMappingSstoreHook(address(target), bytes32(uint256(1)), this.onRevert.selector);
        (success, data) = address(target).call(abi.encodeCall(target.setBalance, (address(0xA11CE), 3)));
        vm.assertFalse(success);
        assertEq(data, abi.encodeWithSignature("Error(string)", "hook revert"));
        assertEq(target.balances(address(0xA11CE)), 0);
    }

    function testCallbackRejectsExternalSpoofing() public {
        bytes32[] memory callbackKeys = new bytes32[](1);
        (bool success,) = address(this)
            .call(
                abi.encodeCall(
                    this.onBalance,
                    (address(target), bytes32(0), bytes32(uint256(1)), callbackKeys, bytes32(0), bytes32(0))
                )
            );
        vm.assertFalse(success);
        assertEq(calls, 0);
    }

    function storeThenRevert(address owner, uint256 value) external {
        target.setBalance(owner, value);
        revert("rollback");
    }

    function onBalance(
        address account,
        bytes32 slot,
        bytes32 root,
        bytes32[] calldata callbackKeys,
        bytes32 oldValue,
        bytes32 newValue
    ) external onlyStorageHook {
        assertEq(account, address(target));
        assertEq(root, bytes32(uint256(1)));
        calls++;
        seenSlot = slot;
        seenOld = oldValue;
        seenNew = newValue;
        ghost[1] = uint256(newValue);
        _setKeys(callbackKeys);
    }

    function onAllowance(address, bytes32, bytes32 root, bytes32[] calldata callbackKeys, bytes32, bytes32)
        external
        onlyStorageHook
    {
        assertEq(root, bytes32(uint256(2)));
        calls++;
        _setKeys(callbackKeys);
    }

    function onReplacement(address, bytes32, bytes32, bytes32[] calldata, bytes32, bytes32) external onlyStorageHook {
        calls = 10;
    }

    function onImplementation(address, bytes32, bytes32, bytes32[] calldata, bytes32, bytes32)
        external
        onlyStorageHook
    {
        implementationCalls++;
    }

    function onIntermediate(address, bytes32, bytes32, bytes32[] calldata, bytes32, bytes32) external onlyStorageHook {
        intermediateCalls++;
    }

    function onRecursive(address, bytes32, bytes32, bytes32[] calldata callbackKeys, bytes32, bytes32)
        external
        onlyStorageHook
    {
        calls++;
        bytes32 slot = target.compute(callbackKeys[0], bytes32(uint256(1)));
        target.directStore(slot, 99);
    }

    function onRevert(address, bytes32, bytes32, bytes32[] calldata, bytes32, bytes32) external onlyStorageHook {
        revert("hook revert");
    }
    function onRaw(address, bytes32, bytes32, bytes32) external onlyStorageHook {}

    function onZeroMemory(address, bytes32 slot, bytes32, bytes32[] calldata callbackKeys, bytes32, bytes32)
        external
        onlyStorageHook
    {
        calls++;
        seenSlot = slot;
        _setKeys(callbackKeys);
    }

    function _setKeys(bytes32[] calldata callbackKeys) internal {
        delete keys;
        for (uint256 i; i < callbackKeys.length; ++i) {
            keys.push(callbackKeys[i]);
        }
    }

    function ghostRoot() internal pure returns (bytes32 root) {
        assembly {
            root := ghost.slot
        }
    }
}
