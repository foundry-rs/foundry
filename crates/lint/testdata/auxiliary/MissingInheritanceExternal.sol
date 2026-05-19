// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

// Stand-in for an interface declared in an external dependency (e.g. node_modules).
// The lint must consider it as a candidate even though it is not in the input set.
interface IExternalThing {
    function doExternalThing() external returns (uint256);
}
