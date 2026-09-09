// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Imported by SolmateSafeTransferLib.sol under an alias: the declared library name is what the
// detector matches, not the name at the call site.
interface ITokenAux {
    function transfer(address to, uint256 amount) external returns (bool);
}

library SafeTransferLib {
    function safeTransfer(ITokenAux token, address to, uint256 amount) internal {
        token.transfer(to, amount);
    }
}
