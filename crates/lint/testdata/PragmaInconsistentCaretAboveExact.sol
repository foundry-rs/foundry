//@compile-flags: --only-lint pragma-inconsistent

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0; //~NOTE: 'pragma solidity ^0.8.0;' conflicts with other version requirements in the project: 0.8.18
pragma solidity 0.8.18; //~NOTE: 'pragma solidity 0.8.18;' conflicts with other version requirements in the project: ^0.8.0

contract Main {}
