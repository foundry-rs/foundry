// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
}

interface IERC20Wrapper {
    function transfer(address to, uint256 amount) external;
    function transferFrom(address from, address to, uint256 amount) external;
}

contract UncheckedTransfer {
    IERC20 public token;
    IERC20Wrapper public tokenWrapper;
    mapping(address => uint256) public balances;

    constructor(address _token) {
        token = IERC20(_token);
        tokenWrapper = IERC20Wrapper(_token);
    }

    // SHOULD FAIL: Unchecked transfer calls
    function uncheckedTransfer(address to, uint256 amount) public {
        IERC20(address(token)).transfer(to, amount); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
        token.transfer(to, amount); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
    }

    function uncheckedTransferFrom(address from, address to, uint256 amount) public {
        IERC20(address(token)).transferFrom(from, to, amount); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
        token.transferFrom(from, to, amount); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
    }

    function uncheckedInLoop(address[] memory recipients, uint256[] memory amounts) public {
        for (uint i = 0; i < recipients.length; i++) {
            IERC20(address(token)).transfer(recipients[i], amounts[i]); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
            token.transfer(recipients[i], amounts[i]); //~WARN: ERC20 'transfer' and 'transferFrom' calls should check the return value
        }
    }

    // SHOULD PASS: Function with same params but NO boolean return
    function proxyCheckedTransfer(address to, uint256 amount) public {
        IERC20Wrapper(address(token)).transfer(to, amount);
        tokenWrapper.transfer(to, amount);
    }

    function proxyCheckedTransferFrom(address from, address to, uint256 amount) public {
        IERC20Wrapper(address(token)).transferFrom(from, to, amount);
        tokenWrapper.transferFrom(from, to, amount);
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
        } else {
            revert("Transfer failed");
        }
    }

    function checkedTransferInRequireWithLogic(address to, uint256 amount) public {
        require(
            amount > 0 && token.transfer(to, amount),
            "Invalid amount or transfer failed"
        );
    }

    function uncheckedApprove(address spender, uint256 amount) public {
        token.approve(spender, amount);
    }
}

library Currency {
    function transfer(address currency, address to, uint256 amount) internal {
        // transfer and check output internally
    }
    function transferFrom(address currency, address from, address to, uint256 amount) internal {
        // transfer and check output internally
    }
}

contract UncheckedTransferUsingCurrencyLib {
    using Currency for address;

    address public token;
    mapping(address => uint256) public balances;

    constructor(address _token) {
        token = _token;
    }

    // SHOULD PASS: Function with same params but NO boolean return
    function currencyTransfer(address to, uint256 amount) public {
        token.transfer(to, amount);
        token.transfer(to, amount);
    }

    function currencyTransferFrom(address from, address to, uint256 amount) public {
        token.transferFrom(from, to, amount);
        token.transferFrom(from, to, amount);
    }
}
