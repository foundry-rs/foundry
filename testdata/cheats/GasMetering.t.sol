// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract B {
    function a() public returns (uint256) {
        return 100;
    }
}

contract GasMeteringTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testGasMetering() public {
        uint256 gas_start = gasleft();

        addInLoop();

        uint256 gas_end_normal = gas_start - gasleft();

        cheats.pauseGasMetering();
        uint256 gas_start_not_metered = gasleft();

        addInLoop();

        uint256 gas_end_not_metered = gas_start_not_metered - gasleft();
        cheats.resumeGasMetering();

        uint256 gas_start_metered = gasleft();

        addInLoop();

        uint256 gas_end_resume_metered = gas_start_metered - gasleft();

        assertEq(gas_end_normal, gas_end_resume_metered);
        assertEq(gas_end_not_metered, 0);
    }

    function testGasMeteringExternal() public {
        B b = new B();
        uint256 gas_start = gasleft();

        b.a();

        uint256 gas_end_normal = gas_start - gasleft();

        cheats.pauseGasMetering();
        uint256 gas_start_not_metered = gasleft();

        b.a();

        uint256 gas_end_not_metered = gas_start_not_metered - gasleft();
        cheats.resumeGasMetering();

        uint256 gas_start_metered = gasleft();

        b.a();

        uint256 gas_end_resume_metered = gas_start_metered - gasleft();

        assertEq(gas_end_normal, gas_end_resume_metered);
        assertEq(gas_end_not_metered, 0);
    }

    function testGasMeteringContractCreate() public {
        cheats.pauseGasMetering();
        uint256 gas_start_not_metered = gasleft();

        B b = new B();

        uint256 gas_end_not_metered = gas_start_not_metered - gasleft();
        cheats.resumeGasMetering();

        assertEq(gas_end_not_metered, 0);
    }

    function addInLoop() internal returns (uint256) {
        uint256 b;
        for (uint256 i; i < 10000; i++) {
            b + i;
        }
        return b;
    }
}
