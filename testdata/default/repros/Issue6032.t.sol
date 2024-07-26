// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6032
contract Issue6032Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testEtchFork() public {
        // Deploy initial contract
        Counter counter = new Counter();
        counter.setNumber(42);

        address counterAddress = address(counter);
        // Enter the fork
        vm.createSelectFork("mainnet");
        assert(counterAddress.code.length > 0);
        // `Counter` is not deployed on the fork, which is expected.

        // Etch the contract into the fork.
        bytes memory code = vm.getDeployedCode("Issue6032.t.sol:CounterEtched");
        vm.etch(counterAddress, code);
        //  `Counter` is now deployed on the fork.
        assert(counterAddress.code.length > 0);

        // Now we can etch the code, but state will remain.
        CounterEtched counterEtched = CounterEtched(counterAddress);
        assertEq(counterEtched.numberHalf(), 21);
    }
}

contract Counter {
    uint256 public number;

    function setNumber(uint256 newNumber) public {
        number = newNumber;
    }
}

contract CounterEtched {
    uint256 public number;

    function numberHalf() public view returns (uint256) {
        return number / 2;
    }
}
