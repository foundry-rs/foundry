//@compile-flags: --only-lint incorrect-strict-equality

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
}

struct Account {
    uint256 balance;
    address owner;
}

struct Holder {
    Account inner;
    address operator;
}

library TokenUtils {
    function balanceOf(IERC20 token, address account) internal view returns (uint256) {
        return token.balanceOf(account);
    }
}

contract IncorrectStrictEquality {
    IERC20 public token;
    uint256 public threshold;
    address public recipient;
    Account public account;
    Holder public holder;
    address[] public holders;
    mapping(uint256 => address) public holderOf;

    constructor(address _token) {
        token = IERC20(_token);
    }

    function getRecipient() internal view returns (address) {
        return recipient;
    }

    function consume(uint256 x) internal pure returns (uint256) {
        return x;
    }

    // SHOULD FAIL:

    function ethBalanceEq() public view returns (bool) {
        return address(this).balance == 1 ether; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function ethBalanceNe() public view returns (bool) {
        return address(this).balance != 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function ethBalanceOnRight() public view returns (bool) {
        return 1 ether == address(this).balance; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function ethBalanceInArith() public view returns (bool) {
        return address(this).balance + 1 == 100 ether; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function tokenBalanceEq() public view returns (bool) {
        return token.balanceOf(address(this)) == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function tokenBalanceNe() public view returns (bool) {
        return token.balanceOf(msg.sender) != threshold; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function tokenBalanceOnRight() public view returns (bool) {
        return 0 == token.balanceOf(address(this)); //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function tokenBalanceInArith() public view returns (bool) {
        return threshold == token.balanceOf(address(this)) - 1; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function inRequireEthBalance() public view {
        require(address(this).balance == 100 ether, "wrong balance"); //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function inRequireTokenBalance() public view {
        require(token.balanceOf(address(this)) == 0, "not empty"); //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function inIfEthBalance() public view returns (uint256) {
        if (address(this).balance == 0) { //~WARN: dangerous strict equality check on an externally-influenced value
            return 1;
        }
        return 0;
    }

    function addressVarBalance() public view returns (bool) {
        return recipient.balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function payableBalance(address payable a) public view returns (bool) {
        return a.balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function payableCastBalance(address a) public view returns (bool) {
        return payable(a).balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function msgSenderBalance() public view returns (bool) {
        return msg.sender.balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function txOriginBalance() public view returns (bool) {
        return tx.origin.balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function blockCoinbaseBalance() public view returns (bool) {
        return block.coinbase.balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function structFieldAddressBalance() public view returns (bool) {
        return account.owner.balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function nestedStructFieldAddressBalance() public view returns (bool) {
        return holder.operator.balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function arrayElementBalance(uint256 i) public view returns (bool) {
        return holders[i].balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function mappingValueBalance(uint256 i) public view returns (bool) {
        return holderOf[i].balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function functionReturnBalance() public view returns (bool) {
        return getRecipient().balance == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function ternaryBalance(bool flag) public view returns (bool) {
        return (flag ? address(this).balance : 0) == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    function callArgBalance() public view returns (bool) {
        return consume(address(this).balance) == 0; //~WARN: dangerous strict equality check on an externally-influenced value
    }

    // SHOULD PASS:

    function ethBalanceGe() public view returns (bool) {
        return address(this).balance >= 1 ether;
    }

    function ethBalanceLe() public view returns (bool) {
        return address(this).balance <= 1 ether;
    }

    function ethBalanceGt() public view returns (bool) {
        return address(this).balance > 0;
    }

    function tokenBalanceGe() public view returns (bool) {
        return token.balanceOf(address(this)) >= threshold;
    }

    function plainUintEq(uint256 x) public view returns (bool) {
        return x == threshold;
    }

    function msgSenderEq(address account_) public view returns (bool) {
        return msg.sender == account_;
    }

    // `msg.value` is intentionally not flagged: exact payment validation is a normal
    // pattern (see `docs/incorrect-strict-equality.md`).
    function msgValueEq() public payable returns (bool) {
        return msg.value == 1 ether;
    }

    // Struct field named `balance` on a non-address type, must NOT trigger the lint.
    function structBalanceEq() public view returns (bool) {
        return account.balance == 0;
    }

    function localStructBalanceEq() public view returns (bool) {
        Account memory a;
        return a.balance == 100;
    }

    // Static library call named `balanceOf` must NOT trigger the lint.
    function libraryBalanceOf() public view returns (bool) {
        return TokenUtils.balanceOf(token, address(this)) == 0;
    }
}
