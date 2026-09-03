// SPDX-License-Identifier: MIT
pragma solidity ^0.8.27;

// The import edge back to the fixture is what used to leak its raw items into this file's
// namespace: only `Errors` is actually bound here, so the fixture's `error Hidden()` must
// not be reachable through `H.`.
import {Errors} from "../UnusedError.sol";

library Hidden {
    function id() internal pure returns (uint256) {
        return 1;
    }
}
