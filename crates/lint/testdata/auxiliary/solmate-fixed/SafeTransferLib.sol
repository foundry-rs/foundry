// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// A same-name `SafeTransferLib` under a path whose component is `solmate-fixed`, not `solmate`.
// The substring "solmate" appears in the path, so a substring provenance check would wrongly
// flag it; a path-component check must treat it as unrelated third-party/local code.

interface IToken {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
}

library SafeTransferLib {
    function safeTransfer(IToken token, address to, uint256 amount) internal {
        require(token.transfer(to, amount), "transfer failed");
    }

    function safeTransferFrom(IToken token, address from, address to, uint256 amount) internal {
        require(token.transferFrom(from, to, amount), "transferFrom failed");
    }

    function safeApprove(IToken token, address spender, uint256 amount) internal {
        require(token.approve(spender, amount), "approve failed");
    }
}
