//@compile-flags: --only-lint pragma-inconsistent

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0; //~NOTE: 2 different Solidity pragma version requirements are used: 0.8.18, ^0.8.0
pragma solidity 0.8.18;

contract Main {}
