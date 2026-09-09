//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract InlineAssembly {
    function bare() public view returns (uint256 id) {
        assembly { //~NOTE: inline assembly used; review for memory safety and side effects
            id := chainid()
        }
    }

    function withMemorySafe() public view returns (uint256 size) {
        assembly ("memory-safe") { //~NOTE: inline assembly (declared memory-safe); review business logic and side effects
            size := extcodesize(address())
        }
    }

    function withDialectAndMemorySafe() public view returns (uint256 ptr) {
        assembly "evmasm" ("memory-safe") { //~NOTE: inline assembly (declared memory-safe); review business logic and side effects
            ptr := mload(0x40)
        }
    }

    function withNatspecMemorySafe() public view returns (uint256 v) {
        /// @solidity memory-safe-assembly
        assembly { //~NOTE: inline assembly (declared memory-safe); review business logic and side effects
            v := chainid()
        }
    }

    function withNatspecMemorySafeAndOtherDocs() public view returns (uint256 v) {
        /// @notice does a thing
        /// @solidity memory-safe-assembly
        assembly { //~NOTE: inline assembly (declared memory-safe); review business logic and side effects
            v := gas()
        }
    }

    function plainCommentDoesNotCount() public view returns (uint256 v) {
        // solidity memory-safe-assembly
        assembly { //~NOTE: inline assembly used; review for memory safety and side effects
            v := chainid()
        }
    }

    function nestedInControlFlow(bool flag) public view returns (uint256 v) {
        if (flag) {
            assembly { //~NOTE: inline assembly used; review for memory safety and side effects
                v := gas()
            }
        }

        for (uint256 i = 0; i < 1; ++i) {
            assembly { //~NOTE: inline assembly used; review for memory safety and side effects
                v := add(v, 1)
            }
        }
    }

    function nestedInUnchecked(uint256 x) public pure returns (uint256 v) {
        unchecked {
            v = x + 1;
            assembly { //~NOTE: inline assembly used; review for memory safety and side effects
                v := add(v, 1)
            }
        }
    }

    function nestedInTryCatch() public returns (uint256 v) {
        try this.bare() returns (uint256) {
            assembly { //~NOTE: inline assembly used; review for memory safety and side effects
                v := 1
            }
        } catch {
            assembly { //~NOTE: inline assembly used; review for memory safety and side effects
                v := 2
            }
        }
    }

    function suppressed() public view returns (uint256 id) {
        // forge-lint: disable-next-line(inline-assembly)
        assembly {
            id := chainid()
        }
    }

    modifier guarded() {
        assembly { //~NOTE: inline assembly used; review for memory safety and side effects
            if iszero(caller()) { revert(0, 0) }
        }
        _;
    }

    function suppressedRegion() public view returns (uint256 a, uint256 b) {
        // forge-lint: disable-start(inline-assembly)
        assembly {
            a := chainid()
        }
        assembly ("memory-safe") {
            b := gas()
        }
        // forge-lint: disable-end(inline-assembly)
    }

    function noAssembly() public pure returns (uint256) {
        return 42;
    }
}
