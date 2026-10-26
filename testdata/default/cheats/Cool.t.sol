// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract CoolTest is Test {
    uint256 public slot0 = 1;

    function testCool_SLOAD_normal() public {
        uint256 startGas;
        uint256 endGas;
        uint256 val;
        uint256 beforeCoolGas;
        uint256 noCoolGas;

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        beforeCoolGas = startGas - endGas;

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        noCoolGas = startGas - endGas;

        assertGt(beforeCoolGas, noCoolGas);
    }

    function testCool_SLOAD() public {
        uint256 startGas;
        uint256 endGas;
        uint256 val;
        uint256 beforeCoolGas;
        uint256 afterCoolGas;
        uint256 noCoolGas;

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        beforeCoolGas = startGas - endGas;

        vm.cool(address(this));

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        afterCoolGas = startGas - endGas;

        assertEq(beforeCoolGas, afterCoolGas);

        startGas = gasleft();
        val = slot0;
        endGas = gasleft();
        noCoolGas = startGas - endGas;

        assertGt(beforeCoolGas, noCoolGas);
    }
}

contract CoolTarget {
    uint256 public value;

    function set(uint256 newValue) external {
        value = newValue;
    }
}

/// forge-config: default.isolate = false
contract CoolPersistenceTest is Test {
    CoolTarget preexisting = new CoolTarget();
    CoolTarget created;

    function setUp() public {
        created = new CoolTarget();
        created.set(1);
        preexisting.set(2);
        vm.cool(address(created));
        vm.cool(address(preexisting));
    }

    function testCoolKeepsSetUpState() public {
        assertEq(preexisting.value(), 2);
        assertEq(created.value(), 1);
    }

    function testCool_EXTCODESIZE() public {
        address target = address(preexisting);
        uint256 size = target.code.length;
        assertGt(size, 0);

        vm.cool(target);

        uint256 gasUsed;
        uint256 cooledSize;
        assembly {
            let start := gas()
            cooledSize := extcodesize(target)
            gasUsed := sub(start, gas())
        }

        assertEq(cooledSize, size);
        assertGe(gasUsed, 2600);
    }

    function balanceGas(address target) internal view returns (uint256 gasUsed) {
        assembly {
            let start := gas()
            let targetBalance := balance(target)
            gasUsed := sub(start, gas())
            // Keeps the `BALANCE` from being optimized out.
            if eq(targetBalance, 0xdeadbeef) { revert(0, 0) }
        }
    }

    // The test contract is the transaction recipient, which is warm from the transaction start.
    function testCool_recipient() public {
        vm.cool(address(this));
        assertGe(balanceGas(address(this)), 2600);
        assertLt(balanceGas(address(this)), 2600);
    }

    // Precompiles and the coinbase stay warm for the whole transaction (EIP-2929, EIP-3651).
    function testCool_keepsPrecompileAndCoinbaseWarm() public {
        vm.cool(address(1));
        vm.cool(block.coinbase);
        assertLt(balanceGas(address(1)), 2600);
        assertLt(balanceGas(block.coinbase), 2600);
    }
}

/// forge-config: default.isolate = true
contract CoolPersistenceIsolatedTest is CoolPersistenceTest {}
