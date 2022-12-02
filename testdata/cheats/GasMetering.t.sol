// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract GasMeteringTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testGasMetering() public {
        uint256 gas_start = gasleft();

        uint256 b;
        for (uint256 i; i < 10000; i++) {
            b + i;
        }

        uint256 gas_end_normal = gas_start - gasleft();


        cheats.stopGasMetering();
        uint256 gas_start_not_metered = gasleft();

         b = 0;
        for (uint256 i; i < 10000; i++) {
            b + i;
        }

        uint256 gas_end_not_metered = gas_start_not_metered - gasleft();
        cheats.startGasMetering();

        assertEq(gas_end_not_metered, 0);
    }
}
