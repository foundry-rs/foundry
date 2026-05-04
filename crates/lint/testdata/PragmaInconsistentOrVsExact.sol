//@compile-flags: --only-lint pragma-inconsistent

// SPDX-License-Identifier: MIT
pragma solidity 0.8.20 || 0.8.21; //~NOTE: 'pragma solidity 0.8.20 || 0.8.21;' conflicts with other version requirements in the project: 0.8.20
pragma solidity 0.8.20; //~NOTE: 'pragma solidity 0.8.20;' conflicts with other version requirements in the project: 0.8.20 || 0.8.21

contract Main {}
