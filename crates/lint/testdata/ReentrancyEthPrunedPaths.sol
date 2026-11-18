//@compile-flags: --only-lint reentrancy-eth reentrancy-balance

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// No function here reads a balance, so the balance analysis skips them; the ETH lint must still
// prune branches that path predicates rule out.
contract ReentrancyEthPrunedPaths {
    uint256 x;

    function infeasibleCall(bool flag) external {
        if (!flag) return;
        uint256 y = x;
        if (!flag) {
            (bool ok,) = msg.sender.call{value: y}("");
            require(ok);
        }
        x = 0;
    }

    function feasibleCall(bool flag) external {
        if (!flag) return;
        uint256 y = x;
        if (flag) {
            (bool ok,) = msg.sender.call{value: y}(""); //~WARN: uncapped ETH transfer can be reentered before `x` is updated
            require(ok);
        }
        x = 0;
    }
}
