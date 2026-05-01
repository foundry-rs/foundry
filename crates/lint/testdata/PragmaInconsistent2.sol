//@compile-flags: --only-lint pragma-inconsistent

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20; //~NOTE: this file uses 'pragma solidity ^0.8.20;', but other files use 0.8.20
pragma solidity 0.8.20; //~NOTE: this file uses 'pragma solidity 0.8.20;', but other files use ^0.8.20

contract Main {}
