// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
}

contract UncheckedTransfer {
    IERC20 public token;
    mapping(address => uint256) public balances;

    constructor(address _token) {
        token = IERC20(_token);
    }

    // SHOULD FAIL: Unchecked transfer calls
    function uncheckedTransfer(address to, uint256 amount) public {
        token.transfer(to, amount); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
    }

    function uncheckedTransferFrom(address from, address to, uint256 amount) public {
        token.transferFrom(from, to, amount); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
    }

    function multipleUnchecked(address to, uint256 amount) public {
        token.transfer(to, amount / 2); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
        token.transfer(to, amount / 2); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
    }

    function uncheckedInLoop(address[] memory recipients, uint256[] memory amounts) public {
        for (uint i = 0; i < recipients.length; i++) {
            token.transfer(recipients[i], amounts[i]); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
        }
    }

    // SHOULD PASS: Properly checked transfer calls
    function checkedTransferWithRequire(address to, uint256 amount) public {
        require(token.transfer(to, amount), "Transfer failed");
    }

    function checkedTransferWithVariable(address to, uint256 amount) public {
        bool success = token.transfer(to, amount);
        require(success, "Transfer failed");
    }

    function checkedTransferFromWithIf(address from, address to, uint256 amount) public {
        bool success = token.transferFrom(from, to, amount);
        if (!success) {
            revert("TransferFrom failed");
        }
    }

    function checkedTransferWithAssert(address to, uint256 amount) public {
        assert(token.transfer(to, amount));
    }

    function checkedTransferInReturn(address to, uint256 amount) public returns (bool) {
        return token.transfer(to, amount);
    }

    function checkedTransferInExpression(address to, uint256 amount) public {
        if (token.transfer(to, amount)) {
            balances[to] += amount;
        }
    }

    function checkedTransferInRequireWithLogic(address to, uint256 amount) public {
        require(
            amount > 0 && token.transfer(to, amount),
            "Invalid amount or transfer failed"
        );
    }

    // Edge case: approve is not a transfer function, should not be flagged
    function uncheckedApprove(address spender, uint256 amount) public {
        token.approve(spender, amount);
    }
}
