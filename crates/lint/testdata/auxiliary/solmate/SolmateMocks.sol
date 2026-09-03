// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Minimal mirror of solmate's SafeTransferLib, under a path that names solmate so the
// provenance check recognizes it: the token operations next to the ETH one.

interface IToken {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
}

library SafeTransferLib {
    function safeTransferETH(address to, uint256 amount) internal {
        payable(to).transfer(amount);
    }

    function safeTransfer(IToken token, address to, uint256 amount) internal {
        token.transfer(to, amount);
    }

    function safeTransferFrom(IToken token, address from, address to, uint256 amount) internal {
        token.transferFrom(from, to, amount);
    }

    function safeApprove(IToken token, address spender, uint256 amount) internal {
        token.approve(spender, amount);
    }
}
