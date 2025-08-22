// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract MappingStorage {
    // Simple mappings only
    mapping(address => uint256) public balances; // Slot 0
    mapping(uint256 => address) public owners; // Slot 1
    mapping(bytes32 => bool) public flags; // Slot 2
    // Nested mapping
    mapping(address => mapping(address => uint256)) public allowances; // Slot 3

    function setBalance(address account, uint256 amount) public {
        balances[account] = amount;
    }

    function setOwner(uint256 tokenId, address owner) public {
        owners[tokenId] = owner;
    }

    function setFlag(bytes32 key, bool value) public {
        flags[key] = value;
    }

    function setAllowance(address owner, address spender, uint256 amount) public {
        allowances[owner][spender] = amount;
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

        // Test the text format output
        string memory stateDiffText = vm.getStateDiff();
        emit log_string("State diff text format:");
        emit log_string(stateDiffText);

        // Verify text format contains the mapping label
        assertContains(
            stateDiffText,
            "balances[0x0000000000000000000000000000000000001234]",
            "Text format should contain mapping label"
        );

        // Verify text format contains the value type
        assertContains(stateDiffText, "uint256", "Text format should contain value type");

        // Verify text format contains decoded values (shown with arrow)
        assertContains(stateDiffText, ": 0", "Text format should contain initial value");
        assertContains(
            stateDiffText, "1000000000000000000000", "Text format should contain new value (1000 ether in wei)"
        );

        // Test JSON format output
        string memory json = vm.getStateDiffJson();
        emit log_string("State diff JSON (simple mapping):");
        emit log_string(json);

        // The JSON should contain the decoded mapping slot with proper label
        assertContains(
            json,
            '"label":"balances[0x0000000000000000000000000000000000001234]"',
            "JSON should contain 'balances[0x0000...1234]' label"
        );

        // Check the type is correctly identified
        assertContains(json, '"type":"mapping(address => uint256)"', "JSON should contain mapping type");

        // Check decoded values
        assertContains(
            json,
            '"decoded":{"previousValue":"0","newValue":"1000000000000000000000"}',
            "JSON should decode balance value correctly (1000 ether = 1000000000000000000000 wei)"
        );

        // Stop recording and verify we have account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length > 0, "Should have account accesses");

        // The AccountAccess structure contains information about storage changes
        // but the label and decoded values are only available in the string/JSON outputs
        // We've already verified those above
    }

    function testMappingWithDifferentKeyTypes() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Test uint256 key
        mappingStorage.setOwner(12345, address(0x7777));

        // Test bytes32 key
        bytes32 flagKey = keccak256("test_flag");
        mappingStorage.setFlag(flagKey, true);

        // Test text format output first
        string memory stateDiffText = vm.getStateDiff();
        emit log_string("State diff text format (different key types):");
        emit log_string(stateDiffText);

        // Verify text format contains decoded values for uint256 key
        assertContains(stateDiffText, "owners[12345]", "Text format should contain owners mapping with decimal key");
        assertContains(
            stateDiffText,
            "address): 0x0000000000000000000000000000000000000000",
            "Text format should contain initial address value"
        );
        assertContains(
            stateDiffText, "0x0000000000000000000000000000000000007777", "Text format should contain new address value"
        );

        // Verify text format contains decoded values for bytes32 key
        assertContains(stateDiffText, "bool): false", "Text format should contain initial bool value");
        assertContains(stateDiffText, "true", "Text format should contain new bool value");

        // Get state diff JSON
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

    function testNestedMappingStateDiff() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Test case 1: owner1 -> spender1
        address owner1 = address(0x1111);
        address spender1 = address(0x2222);
        mappingStorage.setAllowance(owner1, spender1, 500 ether);

        // Test case 2: same owner (owner1) -> different spender (spender2)
        address spender2 = address(0x3333);
        mappingStorage.setAllowance(owner1, spender2, 750 ether);

        // Test case 3: different owner (owner2) -> different spender (spender3)
        address owner2 = address(0x4444);
        address spender3 = address(0x5555);
        mappingStorage.setAllowance(owner2, spender3, 1000 ether);

        // Test text format output
        string memory stateDiffText = vm.getStateDiff();
        emit log_string("State diff text format (nested mappings):");
        emit log_string(stateDiffText);

        // Verify text format contains nested mapping labels
        assertContains(
            stateDiffText,
            "allowances[0x0000000000000000000000000000000000001111][0x0000000000000000000000000000000000002222]",
            "Text format should contain first nested mapping label"
        );
        assertContains(
            stateDiffText,
            "allowances[0x0000000000000000000000000000000000001111][0x0000000000000000000000000000000000003333]",
            "Text format should contain second nested mapping label"
        );
        // The text format shows the value type (uint256) not the full mapping type
        assertContains(stateDiffText, "uint256): 0", "Text format should contain value type");

        // Verify text format contains decoded values for nested mappings
        assertContains(
            stateDiffText,
            "500000000000000000000",
            "Text format should contain decoded value for owner1->spender1 (500 ether)"
        );
        assertContains(
            stateDiffText,
            "750000000000000000000",
            "Text format should contain decoded value for owner1->spender2 (750 ether)"
        );
        assertContains(
            stateDiffText,
            "1000000000000000000000",
            "Text format should contain decoded value for owner2->spender3 (1000 ether)"
        );

        // Test JSON format output
        string memory json = vm.getStateDiffJson();
        emit log_string("State diff JSON (nested mapping - multiple entries):");
        emit log_string(json);

        // Check that all three nested mapping entries are correctly decoded

        // Entry 1: owner1 -> spender1
        assertContains(
            json,
            '"label":"allowances[0x0000000000000000000000000000000000001111][0x0000000000000000000000000000000000002222]"',
            "Should contain first nested mapping label (owner1 -> spender1)"
        );
        assertContains(
            json, '"newValue":"500000000000000000000"', "Should have correct value for owner1 -> spender1 (500 ether)"
        );

        // Entry 2: owner1 -> spender2 (same owner, different spender)
        assertContains(
            json,
            '"label":"allowances[0x0000000000000000000000000000000000001111][0x0000000000000000000000000000000000003333]"',
            "Should contain second nested mapping label (owner1 -> spender2)"
        );
        assertContains(
            json, '"newValue":"750000000000000000000"', "Should have correct value for owner1 -> spender2 (750 ether)"
        );

        // Entry 3: owner2 -> spender3 (different owner)
        assertContains(
            json,
            '"label":"allowances[0x0000000000000000000000000000000000004444][0x0000000000000000000000000000000000005555]"',
            "Should contain third nested mapping label (owner2 -> spender3)"
        );
        assertContains(
            json, '"newValue":"1000000000000000000000"', "Should have correct value for owner2 -> spender3 (1000 ether)"
        );

        // Check the type is correctly identified for all entries
        assertContains(
            json, '"type":"mapping(address => mapping(address => uint256))"', "Should contain nested mapping type"
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
