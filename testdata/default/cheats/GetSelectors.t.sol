// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity =0.8.18;

import "utils/Test.sol";

contract TargetContract {
    function foo() external pure returns (uint256) {
        return 1;
    }

    function bar(uint256 x) external pure returns (uint256) {
        return x;
    }

    function baz(address a, uint256 b) external pure returns (bool) {
        return a != address(0) && b > 0;
    }
}

contract GetSelectorsTest is Test {
    function testGetSelectorsByName() public {
        bytes4[] memory selectors = vm.getSelectors("TargetContract");
        assertEq(selectors.length, 3, "should return 3 selectors");

        // Verify each known selector is present.
        bytes4 fooSel = TargetContract.foo.selector;
        bytes4 barSel = TargetContract.bar.selector;
        bytes4 bazSel = TargetContract.baz.selector;

        assertTrue(_contains(selectors, fooSel), "should contain foo selector");
        assertTrue(_contains(selectors, barSel), "should contain bar selector");
        assertTrue(_contains(selectors, bazSel), "should contain baz selector");
    }

    function testGetSelectorsByNameAndVersion() public {
        bytes4[] memory selectors = vm.getSelectors("TargetContract:0.8.18");
        assertEq(selectors.length, 3, "should return 3 selectors");
    }

    function testGetSelectorsByFullPath() public {
        bytes4[] memory selectors = vm.getSelectors("cheats/GetSelectors.t.sol:TargetContract");
        assertEq(selectors.length, 3, "should return 3 selectors");
    }

    function testGetSelectorsUnknownContractReverts() public {
        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getSelectors("ThisContractDoesNotExist");
    }

    function _contains(bytes4[] memory arr, bytes4 val) internal pure returns (bool) {
        for (uint256 i = 0; i < arr.length; i++) {
            if (arr[i] == val) return true;
        }
        return false;
    }
}
