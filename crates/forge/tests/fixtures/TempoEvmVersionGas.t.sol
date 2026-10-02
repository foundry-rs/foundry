// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.20;

import {Test} from "forge-std/Test.sol";

interface EvmVersionVm {
    function setEvmVersion(string calldata version) external;
    function getEvmVersion() external pure returns (string memory);
}

contract TempoEvmVersionGasTest is Test {
    EvmVersionVm constant forks = EvmVersionVm(address(vm));

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
        string memory current = forks.getEvmVersion();
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
