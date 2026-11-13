// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract B {
    function a() public returns (uint256) {
        return 100;
    }
}

contract Refunder {
    uint256 slot = 1;

    function set(uint256 value) public returns (uint256) {
        slot = value;
        return value;
    }
}

contract RefundDriver {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    Refunder refunder = new Refunder();

    function run() public returns (int64) {
        refunder.set(2);
        vm.resetGasMetering();
        // Restoring the original value refunds more than the frame spends, and the frame returns.
        refunder.set(1);
        return vm.lastFrameGas().gasRefunded;
    }
}

contract GasMeteringTest is Test {
    // https://github.com/foundry-rs/foundry/issues/4370
    function testGasMeteringResetClampsRefundOnReturn() public {
        assertEq(new RefundDriver().run(), 0);
    }

    function testGasMetering() public {
        uint256 gas_start = gasleft();

        consumeGas();

        uint256 gas_end_normal = gas_start - gasleft();

        vm.pauseGasMetering();
        uint256 gas_start_not_metered = gasleft();

        consumeGas();

        uint256 gas_end_not_metered = gas_start_not_metered - gasleft();
        vm.resumeGasMetering();

        uint256 gas_start_metered = gasleft();

        consumeGas();

        uint256 gas_end_resume_metered = gas_start_metered - gasleft();

        assertEq(gas_end_normal, gas_end_resume_metered);
        assertEq(gas_end_not_metered, 0);
    }

    function testGasMeteringExternal() public {
        B b = new B();

        uint256 gas_start = gasleft();

        b.a();

        uint256 gas_end_normal = gas_start - gasleft();

        vm.pauseGasMetering();
        uint256 gas_start_not_metered = gasleft();

        b.a();

        uint256 gas_end_not_metered = gas_start_not_metered - gasleft();
        vm.resumeGasMetering();

        uint256 gas_start_metered = gasleft();

        b.a();

        uint256 gas_end_resume_metered = gas_start_metered - gasleft();

        assertEq(gas_end_normal, gas_end_resume_metered);
        assertEq(gas_end_not_metered, 0);
    }

    function testGasMeteringContractCreate() public {
        vm.pauseGasMetering();
        uint256 gas_start_not_metered = gasleft();

        B b = new B();

        uint256 gas_end_not_metered = gas_start_not_metered - gasleft();
        vm.resumeGasMetering();

        assertEq(gas_end_not_metered, 0);
    }

    function consumeGas() internal returns (uint256 x) {
        for (uint256 i; i < 10000; i++) {
            x += i;
        }
    }
}
