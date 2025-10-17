// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.6.2 <0.9.0;
pragma experimental ABIEncoderV2;

import "./DSTest.sol";
import "./Vm.sol";
import "./console.sol";

contract Test is DSTest {
    Vm public constant vm = Vm(HEVM_ADDRESS);
}
