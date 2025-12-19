// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SimpleContract {
    function hello() public pure returns (string memory) {
        return "hello";
    }
}

contract GetCodeProfileTest is Test {
    function testGetCodeWithProfile() public {
        // Verify positive case: SimpleContract profile "default" exists
        bytes memory codeDefault = vm.getCode("SimpleContract:default");
        assertTrue(codeDefault.length > 0, "Should return bytecode for default profile");

        // Verify negative case: SimpleContract profile "paris" does not exist (in this context)
        try vm.getCode("SimpleContract:paris") {
            revert("Should have reverted");
        } catch Error(string memory reason) {
            assertEq(reason, "no matching artifact found");
        } catch {
            // General catch
        }
    }
}
