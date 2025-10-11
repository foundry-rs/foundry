// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Target is Test {
    function setChainId() public {
        vm.chainId(123);
    }
}

contract Issue10586Test is Test {
    Target public target;

    function setUp() public {
        target = new Target();
    }

    function runTest() internal {
        // By default, the chainId is 31337 during testing.
        assertEq(block.chainid, 31337);

        // Call external function to set the chainId to 123.
        target.setChainId();

        // The chainId is set to 123 in the block.
        assertEq(block.chainid, 123);

        // Set the chainId to 100.
        vm.chainId(100);

        // The chainId is set to 100 in the block.
        assertEq(block.chainid, 100);

        // Call the external function again, which will set the chainId to 123.
        target.setChainId();

        // The last call to chainId() will be the one that is set
        // in the block, so it should be 123.
        assertEq(block.chainid, 123);
    }

    function testGetChainIdAfterSet() public {
        runTest();
    }

    /// forge-config: default.isolate = true
    function testGetChainIdAfterSetIsolated() public {
        // Previous test failed with the following error:
        //
        // [FAIL: EvmError: Revert] testGetChainIdAfterSetIsolated() (gas: 30322)
        // Traces:
        // [208460] DefaultTestContract::setUp()
        //     ├─ [170920] → new Target@0xCe71065D4017F316EC606Fe4422e11eB2c47c246
        //     │   └─ ← [Return] 743 bytes of code
        //     └─ ← [Stop]
        //
        // [30322] DefaultTestContract::testGetChainIdAfterSetIsolated()
        //     ├─ [24037] Target::setChainId()
        //     │   ├─ [0] VM::chainId(123)
        //     │   │   └─ ← [Return]
        //     │   └─ ← [Stop]
        //     ├─ [0] VM::chainId(100)
        //     │   └─ ← [Return]
        //     ├─ [0] Target::setChainId()
        //     │   └─ ← [Revert] EvmError: Revert
        //     └─ ← [Revert] EvmError: Revert
        //
        // Suite result: FAILED. 1 passed; 1 failed; 0 skipped; finished in 2.89ms (1.79ms CPU time)

        runTest();
    }
}
