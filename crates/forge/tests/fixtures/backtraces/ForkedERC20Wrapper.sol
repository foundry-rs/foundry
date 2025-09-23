// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}

contract ForkedERC20Wrapper {
    address constant USDC = 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48;

    function transferWithoutBalance(address recipient, uint256 amount) public {
        IERC20(USDC).transfer(recipient, amount);
    }

    function transferFromWithoutApproval(address from, address to, uint256 amount) public {
        IERC20(USDC).transferFrom(from, to, amount);
    }

    function requireNonZeroBalance(address account) public view {
        uint256 balance = IERC20(USDC).balanceOf(account);
        require(balance > 0, "Account has zero USDC balance");
    }

    function nestedFailure() public {
        internalCall();
    }

    function internalCall() internal {
        transferWithoutBalance(address(0xdead), 1000000);
    }
}
