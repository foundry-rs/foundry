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
        assertGt(target.code.length, 0);

        uint256 warmGas;
        uint256 coldGas;
        uint256 warmSize;
        uint256 coldSize;
        assembly {
            let start := gas()
            warmSize := extcodesize(target)
            warmGas := sub(start, gas())
        }

        vm.cool(target);

        assembly {
            let start := gas()
            coldSize := extcodesize(target)
            coldGas := sub(start, gas())
        }

        assertEq(coldSize, warmSize);
        // Stack operations can differ between reads; only the cooled read pays the 2600 cold cost.
        assertLt(warmGas, 2600);
        assertGe(coldGas, 2600);
    }
}

/// forge-config: default.isolate = true
contract CoolPersistenceIsolatedTest is CoolPersistenceTest {}
