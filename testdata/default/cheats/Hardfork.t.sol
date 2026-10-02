// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.20;

import {Test} from "utils/Test.sol";

interface HardforkVm {
    function _expectCheatcodeRevert(bytes calldata reason) external;
    function setEvmVersion(string calldata version) external;
    function getEvmVersion() external pure returns (string memory);
    function setHardfork(string calldata hardfork) external;
    function getHardfork() external pure returns (string memory);
}

contract HardforkTest is Test {
    HardforkVm constant forks = HardforkVm(address(vm));

    function testEthereumHardforkAndLegacyVersionAgree() public {
        forks.setHardfork("ethereum:cancun");
        assertEq(forks.getHardfork(), "cancun");
        assertEq(forks.getEvmVersion(), "cancun");
        forks.setEvmVersion("shanghai");
        assertEq(forks.getHardfork(), "shanghai");
        forks.setHardfork("cancun");
        assertEq(forks.getEvmVersion(), "cancun");
    }

    function testRejectForeignHardforkWithoutChangingVersion() public {
        forks.setHardfork("cancun");
        forks._expectCheatcodeRevert(bytes("invalid hardfork tempo:T7 for the active network"));
        forks.setHardfork("tempo:T7");
        assertEq(forks.getHardfork(), "cancun");
    }
}
