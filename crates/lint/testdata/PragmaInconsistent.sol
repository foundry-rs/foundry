//@compile-flags: --only-lint pragma-inconsistent

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0; //~NOTE: this file uses 'pragma solidity ^0.8.0;', but other files use 0.8.18
pragma solidity 0.8.18; //~NOTE: this file uses 'pragma solidity 0.8.18;', but other files use ^0.8.0

contract Main {}
