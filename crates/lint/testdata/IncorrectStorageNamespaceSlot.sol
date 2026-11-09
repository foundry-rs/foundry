// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

// Tests for the `incorrect-storage-namespace-slot` lint, which flags inline assembly that points a
// namespaced struct at a constant slot other than the root of its ERC-7201 namespace.

contract Correct {
    /// @custom:storage-location erc7201:openzeppelin.storage.Initializable
    struct InitializableStorage {
        uint64 initialized;
        bool initializing;
    }

    bytes32 private constant INITIALIZABLE_STORAGE =
        0xf0c57e16840df040f15088dc2f81fe391c3923bec73e23a9662efc9c229c6a00;

    function _getInitializableStorage() private pure returns (InitializableStorage storage $) {
        assembly {
            $.slot := INITIALIZABLE_STORAGE
        }
    }
}

contract WrongConstant {
    /// @custom:storage-location erc7201:openzeppelin.storage.ERC20
    struct ERC20Storage {
        mapping(address => uint256) balances;
    }

    // The root of `openzeppelin.storage.ERC721`, copied from another contract.
    bytes32 private constant ERC20_STORAGE =
        0x80bb2b638cc20bc4d0a60d66940f3ab4a00c1d7b313497ca82fb0b4ab0079300;

    function _getERC20Storage() private pure returns (ERC20Storage storage $) {
        assembly {
            $.slot := ERC20_STORAGE //~WARN: storage slot does not match the struct's ERC-7201 namespace
        }
    }
}

contract SharedConstant {
    /// @custom:storage-location erc7201:example.main
    struct MainStorage {
        uint256 value;
    }

    /// @custom:storage-location erc7201:example.other
    struct OtherStorage {
        uint256 value;
    }

    bytes32 private constant MAIN_STORAGE =
        0x183a6125c38840424c4a85fa12bab2ab606c4b6d0e7cc73c0c06ba5300eab500;

    function _main() private pure returns (MainStorage storage $) {
        assembly {
            $.slot := MAIN_STORAGE
        }
    }

    function _other() private pure returns (OtherStorage storage $) {
        assembly {
            // The suggestion must not change MAIN_STORAGE, which is correct for _main.
            $.slot := MAIN_STORAGE //~WARN: storage slot does not match the struct's ERC-7201 namespace
        }
    }
}

contract NarrowConstant {
    /// @custom:storage-location erc7201:openzeppelin.storage.ERC20
    struct ERC20Storage {
        uint256 value;
    }

    bytes4 private constant STORAGE = 0x52c63247;

    function get() private pure returns (ERC20Storage storage $) {
        assembly {
            // The suggestion must not put a 32-byte root into a bytes4 initializer.
            $.slot := STORAGE //~WARN: storage slot does not match the struct's ERC-7201 namespace
        }
    }
}

library LiteralSlot {
    /// @custom:storage-location erc7201:example.library
    struct Data {
        uint256 value;
    }

    function get() internal pure returns (Data storage $) {
        assembly {
            $.slot := 0x1234 //~WARN: storage slot does not match the struct's ERC-7201 namespace
        }
    }
}

contract NotChecked {
    /// @custom:storage-location erc7201:example.main
    struct MainStorage {
        uint256 value;
    }

    struct Plain {
        uint256 value;
    }

    bytes32 private constant SOME_SLOT =
        0x0000000000000000000000000000000000000000000000000000000000000001;

    bytes32 private constant COMPUTED_SLOT =
        keccak256(abi.encode(uint256(keccak256("example.wrong")) - 1)) & ~bytes32(uint256(0xff));

    // Slots that are not compile-time constants are not checked.
    function _computed(bytes32 slot) private pure returns (MainStorage storage $) {
        assembly {
            $.slot := slot
        }
    }

    // Structs without a namespace annotation are not checked.
    function _plain() private pure returns (Plain storage $) {
        assembly {
            $.slot := SOME_SLOT
        }
    }

    // Computed Solidity locals are not checked, even with a constant initializer.
    function _local() private pure returns (MainStorage storage $) {
        bytes32 slot = COMPUTED_SLOT;
        assembly {
            $.slot := slot
        }
    }

    // Yul locals, expressions, and string literals are not checked.
    function _yul() private pure returns (MainStorage storage $) {
        assembly {
            let slot := SOME_SLOT
            $.slot := slot
            $.slot := add(SOME_SLOT, 1)
            $.slot := "wrong"
        }
    }
}
