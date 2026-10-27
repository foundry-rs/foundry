// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.35;

import "utils/Test.sol";

abstract contract NamespacedBase {
    /// @custom:storage-location erc7201:example.storage.Token
    struct TokenStorage {
        mapping(address => uint256) balances;
        address owner;
        uint256 totalSupply;
        bool paused;
    }

    function _token() internal pure returns (TokenStorage storage $) {
        bytes32 slot = keccak256(abi.encode(uint256(keccak256("example.storage.Token")) - 1)) & ~bytes32(uint256(0xff));
        assembly {
            $.slot := slot
        }
    }
}

// Keeps all of its state in an ERC-7201 namespace, so solc's layout has no entries.
contract NamespacedToken is NamespacedBase {
    function mint(address to, uint256 amount) public {
        TokenStorage storage $ = _token();
        $.balances[to] += amount;
        $.totalSupply += amount;
    }

    function setOwner(address owner, bool paused) public {
        TokenStorage storage $ = _token();
        $.owner = owner;
        $.paused = paused;
    }
}

contract StateDiffErc7201Test is Test {
    NamespacedToken internal token;

    function setUp() public {
        token = new NamespacedToken();
    }

    function testNamespacedFields() public {
        vm.startStateDiffRecording();
        token.setOwner(address(0x1234), true);
        string memory json = vm.getStateDiffJson();

        assertTrue(vm.contains(json, '"label":"example.storage.Token.owner"'));
        assertTrue(vm.contains(json, '"label":"example.storage.Token.paused"'));
        assertTrue(vm.contains(json, '"newValue":"0x0000000000000000000000000000000000001234"'));
        assertTrue(vm.contains(json, '"newValue":"true"'));
    }

    function testNamespacedMapping() public {
        vm.startStateDiffRecording();
        token.mint(address(0x1234), 1000);
        string memory json = vm.getStateDiffJson();

        assertTrue(
            vm.contains(json, '"label":"example.storage.Token.balances[0x0000000000000000000000000000000000001234]"')
        );
        assertTrue(vm.contains(json, '"label":"example.storage.Token.totalSupply"'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"1000"}'));
    }
}
