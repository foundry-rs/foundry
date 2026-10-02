// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.20;

import {Test} from "forge-std/Test.sol";

interface HardforkVm {
    function _expectCheatcodeRevert(bytes calldata reason) external;
    function setEvmVersion(string calldata version) external;
    function getEvmVersion() external pure returns (string memory);
    function setHardfork(string calldata hardfork) external;
    function getHardfork() external pure returns (string memory);
}

contract TempoHardforkTest is Test {
    HardforkVm constant forks = HardforkVm(address(vm));

    function deploy(bytes memory initcode) external returns (uint256 used) {
        address deployed;
        uint256 start = gasleft();
        assembly {
            deployed := create(0, add(initcode, 32), mload(initcode))
        }
        used = start - gasleft();
        require(deployed != address(0), "deployment failed");
    }

    function checkGas(bytes memory initcode) internal {
        uint256 before = this.deploy(initcode);
        string memory current = forks.getHardfork();
        forks.setHardfork(string.concat("tempo:", current));
        assertEq(forks.getHardfork(), current);
        assertEq(this.deploy(initcode), before, "setHardfork changed gas");
        forks.setEvmVersion(current);
        assertEq(this.deploy(initcode), before, "setEvmVersion changed gas");
    }

    function testCodeDepositGas() public {
        // Return 24,000 zero bytes of runtime code, as in issue #17276.
        checkGas(hex"615dc05ff3");
    }

    function testNewStorageSlotGas() public {
        checkGas(hex"60015f5500");
    }

    function testEmptyAccountGas() public {
        checkGas(hex"00");
    }

    function testRejectedRequestsPreserveExecution() public {
        string memory current = forks.getHardfork();
        uint256 before = this.deploy(hex"615dc05ff3");
        forks._expectCheatcodeRevert(bytes("invalid hardfork osaka for the active network"));
        forks.setHardfork("osaka");
        forks._expectCheatcodeRevert(
            bytes(
                "changing Tempo hardforks during execution is unsupported; set hardfork = \"tempo:<revision>\" before execution so instructions, precompiles, and gas parameters agree"
            )
        );
        forks.setHardfork("tempo:T2");
        assertEq(forks.getHardfork(), current);
        assertEq(forks.getEvmVersion(), current);
        assertEq(this.deploy(hex"615dc05ff3"), before);
    }

    function testLegacyEthereumAliasGas() public {
        // Repeating the legacy mapping must preserve the selected network's gas schedule.
        forks.setEvmVersion("osaka");
        uint256 before = this.deploy(hex"615dc05ff3");
        forks.setEvmVersion("osaka");
        assertEq(this.deploy(hex"615dc05ff3"), before);
        assertGt(before, 24_000_000);
    }

    function testLegacyNativeRevisionChange() public {
        forks.setEvmVersion("tempo:T2");
        assertEq(forks.getEvmVersion(), "t2");
        uint256 before = this.deploy(hex"615dc05ff3");
        forks.setEvmVersion("T2");
        assertEq(this.deploy(hex"615dc05ff3"), before);
        assertGt(before, 24_000_000);
    }
}
