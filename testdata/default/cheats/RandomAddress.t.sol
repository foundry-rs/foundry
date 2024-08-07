// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RandomAddress is DSTest {
  Vm constant vm = Vm(HEVM_ADDRESS);

  function testRandomAddress() public {
    vm.randomAddress();
  }

  function testDeterministicRandomAddress() public {
    address alice = vm.randomAddress();
    address bob = vm.randomAddress();
    assertEq(alice, bob);
  }
}