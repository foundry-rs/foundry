// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "../lib/ds-test/src/test.sol";
import "./Vm.sol";

contract CoolTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
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
