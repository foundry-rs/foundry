// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract MappingStorage {
    // Simple mappings only
    mapping(address => uint256) public balances; // Slot 0
    mapping(uint256 => address) public owners; // Slot 1
    mapping(bytes32 => bool) public flags; // Slot 2

    function setBalance(address account, uint256 amount) public {
        balances[account] = amount;
    }

    function setOwner(uint256 tokenId, address owner) public {
        owners[tokenId] = owner;
    }

    function setFlag(bytes32 key, bool value) public {
        flags[key] = value;
    }
}

contract StateDiffMappingsTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    MappingStorage public mappingStorage;

    function setUp() public {
        mappingStorage = new MappingStorage();
    }

    function testSimpleMappingStateDiff() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Modify a simple mapping
        address testAccount = address(0x1234);
        mappingStorage.setBalance(testAccount, 1000 ether);

        // Get state diff as JSON for detailed inspection
        string memory json = vm.getStateDiffJson();

        // Debug: log the JSON for inspection
        emit log_string("State diff JSON (simple mapping):");
        emit log_string(json);

        // The JSON should contain the decoded mapping slot with proper label
        // Expected: "balances[0x0000...1234]" in the label field
        assertContains(
            json,
            '"label":"balances[0x0000000000000000000000000000000000001234]"',
            "Should contain 'balances[0x0000...1234]' label"
        );

        // Check the type is correctly identified
        assertContains(json, '"type":"mapping(address => uint256)"', "Should contain mapping type");

        // Check decoded values
        assertContains(
            json,
            '"decoded":{"previousValue":"0","newValue":"1000000000000000000000"}',
            "Should decode balance value correctly (1000 ether = 1000000000000000000000 wei)"
        );

        // Also test text format
        string memory stateDiff = vm.getStateDiff();
        assertContains(
            stateDiff,
            "balances[0x0000000000000000000000000000000000001234]",
            "Text format should contain mapping label"
        );

        // Stop recording and verify we have account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length > 0, "Should have account accesses");
    }

    function testMappingWithDifferentKeyTypes() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Test uint256 key
        mappingStorage.setOwner(12345, address(0x7777));

        // Test bytes32 key
        bytes32 flagKey = keccak256("test_flag");
        mappingStorage.setFlag(flagKey, true);

        // Get state diff
        string memory json = vm.getStateDiffJson();

        // Debug: log the JSON for inspection
        emit log_string("State diff JSON (different key types):");
        emit log_string(json);

        // Check uint256 key mapping
        assertContains(json, '"label":"owners[12345]"', "Should contain owners mapping with uint256 key");
        assertContains(json, '"type":"mapping(uint256 => address)"', "Should contain uint256=>address mapping type");
        assertContains(
            json,
            '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000007777"}',
            "Should decode owner address correctly"
        );

        // Check bytes32 key mapping - the key will be shown as hex
        assertContains(json, '"label":"flags[', "Should contain flags mapping label");
        assertContains(json, '"type":"mapping(bytes32 => bool)"', "Should contain bytes32=>bool mapping type");
        assertContains(
            json, '"decoded":{"previousValue":"false","newValue":"true"}', "Should decode flag bool value correctly"
        );

        // Stop recording
        vm.stopAndReturnStateDiff();
    }

    // Helper function to check if a string contains a substring
    function assertContains(string memory haystack, string memory needle, string memory message) internal pure {
        bytes memory haystackBytes = bytes(haystack);
        bytes memory needleBytes = bytes(needle);

        if (needleBytes.length > haystackBytes.length) {
            revert(message);
        }

        bool found = false;
        for (uint256 i = 0; i <= haystackBytes.length - needleBytes.length; i++) {
            bool isMatch = true;
            for (uint256 j = 0; j < needleBytes.length; j++) {
                if (haystackBytes[i + j] != needleBytes[j]) {
                    isMatch = false;
                    break;
                }
            }
            if (isMatch) {
                found = true;
                break;
            }
        }

        if (!found) {
            revert(message);
        }
    }
}
