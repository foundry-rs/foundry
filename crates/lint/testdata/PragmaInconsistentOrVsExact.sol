//@compile-flags: --only-lint pragma-inconsistent

// SPDX-License-Identifier: MIT
pragma solidity 0.8.20 || 0.8.21; //~NOTE: 2 different Solidity pragma version requirements are used: 0.8.20, 0.8.20 || 0.8.21
pragma solidity 0.8.20;

contract Main {}
