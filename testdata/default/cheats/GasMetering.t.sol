// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract B {
    function a() public returns (uint256) {
        return 100;
    }
}

contract GasMeteringTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    function testGasMetering() public {
        uint256 gasStart = gasleft();

        consumeGas();

        uint256 gasEndNormal = gasStart - gasleft();

        VM.pauseGasMetering();
        uint256 gasStartNotMetered = gasleft();

        consumeGas();

        uint256 gasEndNotMetered = gasStartNotMetered - gasleft();
        VM.resumeGasMetering();

        uint256 gasStartMetered = gasleft();

        consumeGas();

        uint256 gasEndResumeMetered = gasStartMetered - gasleft();

        assertEq(gasEndNormal, gasEndResumeMetered);
        assertEq(gasEndNotMetered, 0);
    }

    function testGasMeteringExternal() public {
        B b = new B();
        uint256 gasStart = gasleft();

        b.a();

        uint256 gasEndNormal = gasStart - gasleft();

        VM.pauseGasMetering();
        uint256 gasStartNotMetered = gasleft();

        b.a();

        uint256 gasEndNotMetered = gasStartNotMetered - gasleft();
        VM.resumeGasMetering();

        uint256 gasStartMetered = gasleft();

        b.a();

        uint256 gasEndResumeMetered = gasStartMetered - gasleft();

        assertEq(gasEndNormal, gasEndResumeMetered);
        assertEq(gasEndNotMetered, 0);
    }

    function testGasMeteringContractCreate() public {
        VM.pauseGasMetering();
        uint256 gasStartNotMetered = gasleft();

        B b = new B();

        uint256 gasEndNotMetered = gasStartNotMetered - gasleft();
        VM.resumeGasMetering();

        assertEq(gasEndNotMetered, 0);
    }

    function consumeGas() internal returns (uint256 x) {
        for (uint256 i; i < 10000; i++) {
            x += i;
        }
    }
}
