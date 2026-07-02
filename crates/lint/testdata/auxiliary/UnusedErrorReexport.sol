// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

// Re-exports the fixture's error under a new name: a use of `R.Written` in the fixture
// must count as a use of `UsedViaAliasReexport`.
import {UsedViaAliasReexport as Written} from "../UnusedError.sol";
