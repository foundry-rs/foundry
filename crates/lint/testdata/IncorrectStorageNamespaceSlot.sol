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

contract ComputedConstant {
    /// @custom:storage-location erc7201:example.main
    struct MainStorage {
        uint256 value;
    }

    /// @custom:storage-location erc7201:example.other
    struct OtherStorage {
        uint256 value;
    }

    bytes32 private constant MAIN_STORAGE =
        keccak256(abi.encode(uint256(keccak256("example.main")) - 1)) & ~bytes32(uint256(0xff));

    // The namespace string differs from the annotation.
    bytes32 private constant OTHER_STORAGE =
        keccak256(abi.encode(uint256(keccak256("example.others")) - 1)) & ~bytes32(uint256(0xff));

    function _main() private pure returns (MainStorage storage $) {
        assembly {
            $.slot := MAIN_STORAGE
        }
    }

    function _other() private pure returns (OtherStorage storage $) {
        assembly {
            $.slot := OTHER_STORAGE //~WARN: storage slot does not match the struct's ERC-7201 namespace
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

    bytes32 private constant SOME_SLOT = bytes32(uint256(1));

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
}
