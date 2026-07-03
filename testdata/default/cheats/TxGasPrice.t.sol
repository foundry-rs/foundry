// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract TxGasPriceTest is Test {
    function testTxGasPrice() public {
        vm.txGasPrice(10 gwei);
        assertEq(tx.gasprice, 10 gwei);
    }
}

/// `vm.txGasPrice` must remain visible to a *called* contract under
/// `--isolate`/`--gas-report`, where the synthetic inner transaction zeroes
/// `tx.gasprice` for fee accounting. Also asserts that propagating the gas
/// price does not start requiring pranked callers to pre-fund `gas * gasPrice`.
/// Regression test for #7277.
/// forge-config: default.isolate = true
contract IsolatedTxGasPriceTest is Test {
    GasPriceRecorder internal recorder;
    address internal constant ALICE = address(0xA11CE);

    function setUp() public {
        recorder = new GasPriceRecorder();
    }

    function test_txGasPrice_visible_in_called_contract() public {
        vm.txGasPrice(10 gwei);
        recorder.record();
        assertEq(recorder.lastGasPrice(), 10 gwei);
    }

    function test_txGasPrice_pranked_zero_balance_caller_can_call() public {
        assertEq(ALICE.balance, 0, "ALICE should start with zero balance");

        vm.txGasPrice(1);
        vm.startPrank(ALICE);
        recorder.bump();
        vm.stopPrank();

        assertEq(recorder.bumps(), 1);
    }
}

contract GasPriceRecorder {
    uint256 public lastGasPrice;
    uint256 public bumps;

    function record() external {
        lastGasPrice = tx.gasprice;
    }

    function bump() external {
        bumps += 1;
    }
}
