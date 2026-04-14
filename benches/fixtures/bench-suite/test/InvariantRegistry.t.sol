// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {Registry} from "../src/Registry.sol";
import {Vm} from "./Vm.sol";

contract RegistryHandler {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    Registry public registry;

    address[3] public actors = [address(0x1001), address(0x1002), address(0x1003)];
    bytes32[] public registeredKeys;
    uint256 public registerCount;
    uint256 public updateCount;
    uint256 public deactivateCount;

    constructor(Registry _r) {
        registry = _r;
    }

    function register(uint256 actorIdx, uint256 keySeed, uint256 value) external {
        address actor = actors[actorIdx % actors.length];
        bytes32 key = keccak256(abi.encodePacked(keySeed, registerCount));

        vm.prank(actor);
        registry.register(key, value);
        registeredKeys.push(key);
        registerCount++;
    }

    function update(uint256 keyIdx, uint256 newValue) external {
        if (registeredKeys.length == 0) return;
        keyIdx = keyIdx % registeredKeys.length;
        bytes32 key = registeredKeys[keyIdx];

        (address owner,,, bool active) = registry.entries(key);
        if (!active) return;

        vm.prank(owner);
        registry.update(key, newValue);
        updateCount++;
    }

    function deactivate(uint256 keyIdx) external {
        if (registeredKeys.length == 0) return;
        keyIdx = keyIdx % registeredKeys.length;
        bytes32 key = registeredKeys[keyIdx];

        (address owner,,, bool active) = registry.entries(key);
        if (!active) return;

        vm.prank(owner);
        registry.deactivate(key);
        deactivateCount++;
    }
}

contract InvariantRegistryTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    Registry registry;
    RegistryHandler handler;

    function setUp() public {
        registry = new Registry();
        handler = new RegistryHandler(registry);
    }

    function targetContracts() public view returns (address[] memory) {
        address[] memory targets = new address[](1);
        targets[0] = address(handler);
        return targets;
    }

    function invariant_totalEntriesMatchesRegisterCount() public view {
        assert(registry.totalEntries() == handler.registerCount());
    }

    function invariant_registeredKeyHasOwner() public view {
        for (uint256 i = 0; i < handler.registerCount() && i < 50; i++) {
            bytes32 key = handler.registeredKeys(i);
            (address owner,,,) = registry.entries(key);
            assert(owner != address(0));
        }
    }
}
