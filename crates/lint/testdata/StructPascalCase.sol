//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract StructPascalCaseTest {
    struct PascalCase {
        uint256 a;
    }

    struct PascalCAse {
        uint256 a;
    }

    // Already valid PascalCase with a preserved leading underscore - not flagged.
    struct _PascalCase {
        uint256 a;
    }

    struct _otherCase { //~NOTE: struct name is not `PascalCase`
        uint256 a;
    }

    struct pascalCase { //~NOTE: struct name is not `PascalCase`
        uint256 a;
    }

    struct pascalcase { //~NOTE: struct name is not `PascalCase`
        uint256 a;
    }

    struct pascal_case { //~NOTE: struct name is not `PascalCase`
        uint256 a;
    }

    struct PASCAL_CASE { //~NOTE: struct name is not `PascalCase`
        uint256 a;
    }

    struct PASCALCASE { //~NOTE: struct name is not `PascalCase`
        uint256 a;
    }

    // Configured acronym exception ("ERC") - not flagged, with or without preserved underscores.
    struct ERC20Data {
        uint256 a;
    }

    struct _ERC20Data {
        uint256 a;
    }

    struct ERC20Data_ {
        uint256 a;
    }

    struct __ERC20Data { //~NOTE: struct name is not `PascalCase`
        uint256 a;
    }

    struct ERC20Data__ { //~NOTE: struct name is not `PascalCase`
        uint256 a;
    }
}
