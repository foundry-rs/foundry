// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {IToken} from "../solmate/SolmateMocks.sol";

// A same-name library from another package: Solady's SafeTransferLib checks that the token
// has code on the empty-return success path, so it does not have the solmate v6 pitfall.
// Its path does not name solmate, which is what keeps it out of scope.

library SafeTransferLib {
    function safeTransfer(IToken token, address to, uint256 amount) internal {
        require(address(token).code.length > 0, "no code");
        token.transfer(to, amount);
    }
}
