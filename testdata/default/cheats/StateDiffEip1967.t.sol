// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.35;

import "utils/Test.sol";

// Writes the EIP-1967 slots from assembly constants, so solc's layout only covers `version`.
contract Eip1967Proxy {
    uint256 public version;

    constructor(address implementation, address admin, address beacon) {
        assembly {
            sstore(0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc, implementation)
            sstore(0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103, admin)
            sstore(0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50, beacon)
        }
        version = 1;
    }
}

contract StateDiffEip1967Test is Test {
    function testLabelsEip1967Slots() public {
        vm.startStateDiffRecording();
        new Eip1967Proxy(address(0x1234), address(0xA11CE), address(0xBEAC0));
        string memory json = vm.getStateDiffJson();
        string memory text = vm.getStateDiff();

        // Layout-covered slots are unaffected.
        assertTrue(vm.contains(json, '"label":"version","type":"uint256","offset":0,"slot":"0"'));
        assertTrue(
            vm.contains(
                json,
                '"label":"eip1967.proxy.implementation","type":"address","offset":0,"slot":"24440054405305269366569402256811496959409073762505157381672968839269610695612","decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000001234"}'
            )
        );
        assertTrue(
            vm.contains(
                json,
                '"label":"eip1967.proxy.admin","type":"address","offset":0,"slot":"81955473079516046949633743016697847541294818689821282749996681496272635257091","decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x00000000000000000000000000000000000A11cE"}'
            )
        );
        assertTrue(
            vm.contains(
                json,
                '"label":"eip1967.proxy.beacon","type":"address","offset":0,"slot":"74152234768234802001998023604048924213078445070507226371336425913862612794704","decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x00000000000000000000000000000000000bEaC0"}'
            )
        );
        assertTrue(
            vm.contains(
                text,
                unicode"(eip1967.proxy.implementation, address): 0x0000000000000000000000000000000000000000 → 0x0000000000000000000000000000000000001234"
            )
        );
    }

    // An account without any known layout still gets the EIP-1967 labels.
    function testLabelsEip1967SlotsWithoutLayout() public {
        // Init code: sstore(implementationSlot, 0x1234), deploying empty code.
        bytes memory initCode = abi.encodePacked(
            hex"611234", hex"7f", bytes32(0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc), hex"5500"
        );
        vm.startStateDiffRecording();
        address deployed;
        assembly {
            deployed := create(0, add(initCode, 0x20), mload(initCode))
        }
        assertTrue(deployed != address(0));
        string memory json = vm.getStateDiffJson();

        assertTrue(vm.contains(json, '"contract":null'));
        assertTrue(
            vm.contains(
                json,
                '"label":"eip1967.proxy.implementation","type":"address","offset":0,"slot":"24440054405305269366569402256811496959409073762505157381672968839269610695612","decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000001234"}'
            )
        );
    }
}
