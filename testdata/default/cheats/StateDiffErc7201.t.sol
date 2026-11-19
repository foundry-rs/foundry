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

type NamespacedConfig is uint16;

library NamespacedIds {
    type LibId is uint24;
}

// User-defined value types packed between plain fields of a namespace.
contract NamespacedUdvt {
    /// @custom:storage-location erc7201:example.storage.Udvt
    struct UdvtStorage {
        uint32 timestamp; // offset 0
        NamespacedConfig config; // offset 4
        NamespacedIds.LibId libId; // offset 6
        uint64 count; // offset 9
    }

    function _udvt() internal pure returns (UdvtStorage storage $) {
        bytes32 slot = keccak256(abi.encode(uint256(keccak256("example.storage.Udvt")) - 1)) & ~bytes32(uint256(0xff));
        assembly {
            $.slot := slot
        }
    }

    function set() public {
        UdvtStorage storage $ = _udvt();
        $.timestamp = 7;
        $.config = NamespacedConfig.wrap(4);
        $.libId = NamespacedIds.LibId.wrap(0x0a0b0c);
        $.count = 9;
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

    // Namespaced type ids are scoped as `erc7201(<id>)::t_userDefinedValueType(...)`, so user-defined
    // value types must still resolve to an integer of their recorded size.
    function testNamespacedUserDefinedValueTypes() public {
        NamespacedUdvt store = new NamespacedUdvt();

        vm.startStateDiffRecording();
        store.set();
        string memory json = vm.getStateDiffJson();
        emit log_string(json);

        // The file-level type is a member of its own, decoded at its width.
        assertTrue(vm.contains(json, '"type":"(uint32,uint16,uint24,uint64)"'));
        assertTrue(vm.contains(json, '"label":"example.storage.Udvt.config","type":"NamespacedConfig","offset":4'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"4"}'));
        // The library-scoped one is decoded from all three of its bytes, not its lowest.
        assertTrue(vm.contains(json, '"label":"example.storage.Udvt.libId","type":"NamespacedIds.LibId","offset":6'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"658188"}'));

        vm.stopAndReturnStateDiff();
    }
}
