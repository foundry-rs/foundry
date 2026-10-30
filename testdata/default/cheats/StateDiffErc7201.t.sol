// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.35;

import "utils/Test.sol";

abstract contract NamespacedInitializable {
    /// @custom:storage-location erc7201:openzeppelin.storage.Initializable
    struct InitializableStorage {
        uint64 _initialized;
        bool _initializing;
    }

    function _initializable() internal pure returns (InitializableStorage storage $) {
        assembly {
            $.slot := 0xf0c57e16840df040f15088dc2f81fe391c3923bec73e23a9662efc9c229c6a00
        }
    }
}

abstract contract NamespacedBase {
    /// @custom:storage-location erc7201:example.storage.Token
    struct TokenStorage {
        mapping(address => uint256) balances;
        uint256 totalSupply;
        address owner;
        bool paused;
    }

    function _token() internal pure returns (TokenStorage storage $) {
        bytes32 slot = keccak256(abi.encode(uint256(keccak256("example.storage.Token")) - 1)) & ~bytes32(uint256(0xff));
        assembly {
            $.slot := slot
        }
    }
}

// Keeps all of its state in ERC-7201 namespaces, so solc's layout has no entries.
contract NamespacedToken is NamespacedInitializable, NamespacedBase {
    function initialize() public {
        _initializable()._initialized = 1;
    }

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

    function testPackedNamespacedFields() public {
        vm.startStateDiffRecording();
        token.setOwner(address(0x1234), true);
        string memory json = vm.getStateDiffJson();
        string memory text = vm.getStateDiff();

        // Both packed fields are labeled and decoded from their own bytes.
        assertTrue(vm.contains(json, '"label":"example.storage.Token.owner, example.storage.Token.paused"'));
        assertTrue(vm.contains(json, '"type":"(address,bool)"'));
        assertTrue(vm.contains(json, '"label":"example.storage.Token.owner","type":"address","offset":0'));
        assertTrue(
            vm.contains(
                json,
                '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000001234"}'
            )
        );
        assertTrue(vm.contains(json, '"label":"example.storage.Token.paused","type":"bool","offset":20'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"false","newValue":"true"}'));

        assertTrue(
            vm.contains(
                text,
                unicode"example.storage.Token.owner (address): 0x0000000000000000000000000000000000000000 → 0x0000000000000000000000000000000000001234"
            )
        );
        assertTrue(vm.contains(text, unicode"example.storage.Token.paused (bool): false → true"));
    }

    function testInitializableNamespace() public {
        vm.startStateDiffRecording();
        token.initialize();
        string memory json = vm.getStateDiffJson();

        assertTrue(
            vm.contains(
                json,
                '"label":"openzeppelin.storage.Initializable._initialized, openzeppelin.storage.Initializable._initializing"'
            )
        );
        assertTrue(
            vm.contains(
                json,
                '"label":"openzeppelin.storage.Initializable._initialized","type":"uint64","offset":0,"slot":"108904022758810753673719992590105913556127789646572562039383141376366747609600","decoded":{"previousValue":"0","newValue":"1"}'
            )
        );
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
