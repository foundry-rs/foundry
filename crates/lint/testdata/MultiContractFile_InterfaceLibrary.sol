//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Interface counts as a contract-like item.
interface I1 {}

// Library is also a contract-like item and it should be counted.
library L1 {} //~NOTE: file contains multiple contracts, interfaces or libraries

// Third contract-like item.
contract C1 {}

