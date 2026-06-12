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

    struct _PascalCase { //~NOTE: structs should use PascalCase
        uint256 a;
    }

    struct pascalCase { //~NOTE: structs should use PascalCase
        uint256 a;
    }

    struct pascalcase { //~NOTE: structs should use PascalCase
        uint256 a;
    }

    struct pascal_case { //~NOTE: structs should use PascalCase
        uint256 a;
    }

    struct PASCAL_CASE { //~NOTE: structs should use PascalCase
        uint256 a;
    }

    struct PASCALCASE { //~NOTE: structs should use PascalCase
        uint256 a;
    }
}
