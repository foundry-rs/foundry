// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "utils/Test.sol";

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

contract StateDiffMappingsTest is Test {
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
        assertTrue(vm.contains(stateDiffText, "balances[0x0000000000000000000000000000000000001234]"));

        // Verify text format contains the value type
        assertTrue(vm.contains(stateDiffText, "uint256"));

        // Verify text format contains decoded values (shown with arrow)
        assertTrue(vm.contains(stateDiffText, ": 0"));
        assertTrue(vm.contains(stateDiffText, "1000000000000000000000"));

        // Test JSON format output
        string memory json = vm.getStateDiffJson();
        emit log_string("State diff JSON (simple mapping):");
        emit log_string(json);

        // The JSON should contain the decoded mapping slot with proper label
        assertTrue(vm.contains(json, '"label":"balances[0x0000000000000000000000000000000000001234]"'));

        // Check the type is correctly identified
        assertTrue(vm.contains(json, '"type":"mapping(address => uint256)"'));

        // Check decoded values
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"1000000000000000000000"}'));

        // Check that the key field is present for simple mapping
        assertTrue(vm.contains(json, '"key":"0x0000000000000000000000000000000000001234"'));

        // Stop recording and verify we have account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length > 0);

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
        assertTrue(vm.contains(stateDiffText, "owners[12345]"));
        assertTrue(vm.contains(stateDiffText, "address): 0x0000000000000000000000000000000000000000"));
        assertTrue(vm.contains(stateDiffText, "0x0000000000000000000000000000000000007777"));

        // Verify text format contains decoded values for bytes32 key
        assertTrue(vm.contains(stateDiffText, "bool): false"));
        assertTrue(vm.contains(stateDiffText, "true"));

        // Get state diff JSON
        string memory json = vm.getStateDiffJson();

        // Debug: log the JSON for inspection
        emit log_string("State diff JSON (different key types):");
        emit log_string(json);

        // Check uint256 key mapping
        assertTrue(vm.contains(json, '"label":"owners[12345]"'));
        assertTrue(vm.contains(json, '"type":"mapping(uint256 => address)"'));
        assertTrue(
            vm.contains(
                json,
                '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000007777"}'
            )
        );

        // Check bytes32 key mapping - the key will be shown as hex
        assertTrue(vm.contains(json, '"label":"flags['));
        assertTrue(vm.contains(json, '"type":"mapping(bytes32 => bool)"'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"false","newValue":"true"}'));

        // Check that the key field is present for uint256 key mapping
        assertTrue(vm.contains(json, '"key":"12345"'));

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
        assertTrue(
            vm.contains(
                stateDiffText,
                "allowances[0x0000000000000000000000000000000000001111][0x0000000000000000000000000000000000002222]"
            )
        );
        assertTrue(
            vm.contains(
                stateDiffText,
                "allowances[0x0000000000000000000000000000000000001111][0x0000000000000000000000000000000000003333]"
            )
        );
        // The text format shows the value type (uint256) not the full mapping type
        assertTrue(vm.contains(stateDiffText, "uint256): 0"));

        // Verify text format contains decoded values for nested mappings
        assertTrue(vm.contains(stateDiffText, "500000000000000000000"));
        assertTrue(vm.contains(stateDiffText, "750000000000000000000"));
        assertTrue(vm.contains(stateDiffText, "1000000000000000000000"));

        // Test JSON format output
        string memory json = vm.getStateDiffJson();
        emit log_string("State diff JSON (nested mapping - multiple entries):");
        emit log_string(json);

        // Check that all three nested mapping entries are correctly decoded

        // Entry 1: owner1 -> spender1
        assertTrue(
            vm.contains(
                json,
                '"label":"allowances[0x0000000000000000000000000000000000001111][0x0000000000000000000000000000000000002222]"'
            )
        );
        assertTrue(vm.contains(json, '"newValue":"500000000000000000000"'));

        // Entry 2: owner1 -> spender2 (same owner, different spender)
        assertTrue(
            vm.contains(
                json,
                '"label":"allowances[0x0000000000000000000000000000000000001111][0x0000000000000000000000000000000000003333]"'
            )
        );
        assertTrue(vm.contains(json, '"newValue":"750000000000000000000"'));

        // Entry 3: owner2 -> spender3 (different owner)
        assertTrue(
            vm.contains(
                json,
                '"label":"allowances[0x0000000000000000000000000000000000004444][0x0000000000000000000000000000000000005555]"'
            )
        );
        assertTrue(vm.contains(json, '"newValue":"1000000000000000000000"'));

        // Check the type is correctly identified for all entries
        assertTrue(vm.contains(json, '"type":"mapping(address => mapping(address => uint256))"'));

        // Check that the keys field is present for nested mappings
        assertTrue(
            vm.contains(
                json,
                '"keys":["0x0000000000000000000000000000000000001111","0x0000000000000000000000000000000000002222"]'
            )
        );

        assertTrue(
            vm.contains(
                json,
                '"keys":["0x0000000000000000000000000000000000001111","0x0000000000000000000000000000000000003333"]'
            )
        );

        assertTrue(
            vm.contains(
                json,
                '"keys":["0x0000000000000000000000000000000000004444","0x0000000000000000000000000000000000005555"]'
            )
        );

        // Stop recording
        vm.stopAndReturnStateDiff();
    }
}
