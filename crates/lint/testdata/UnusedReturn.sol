//@compile-flags: --only-lint unused-return

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IOracle {
    function getPrice(address token) external returns (uint256);
    function latest(address token) external returns (uint256, bool);
    function update() external returns (bool);
    function noReturn() external;
}

// Same name+arity as IOracle.getPrice but one overload has no return value.
interface IOracleOverloaded {
    function getPrice(address token) external returns (uint256);
    function getPrice(uint256 id) external; // no return, makes getPrice ambiguous
}

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

contract UnusedReturn {
    IOracle public oracle;
    IOracleOverloaded public oracleOverloaded;
    IERC20 public token;

    constructor(address _oracle, address _token) {
        oracle = IOracle(_oracle);
        oracleOverloaded = IOracleOverloaded(_oracle);
        token = IERC20(_token);
    }

    // SHOULD FAIL: uint256 return discarded
    function bad1(address t) external {
        oracle.getPrice(t); //~WARN: Return value of an external call is not used
    }

    // SHOULD FAIL: bool return discarded (non-ERC20 function)
    function bad2() external {
        oracle.update(); //~WARN: Return value of an external call is not used
    }

    // SHOULD FAIL: explicit interface cast, IOracle(addr).getPrice(t)
    function bad3(address oracleAddr, address t) external {
        IOracle(oracleAddr).getPrice(t); //~WARN: Return value of an external call is not used
    }

    // SHOULD PASS: return value stored in local variable
    function good1(address t) external returns (uint256) {
        uint256 price = oracle.getPrice(t);
        return price;
    }

    // SHOULD PASS: return value used directly in expression
    function good2(address t) external returns (uint256) {
        return oracle.getPrice(t);
    }

    // SHOULD PASS: function has no return value
    function good3() external {
        oracle.noReturn();
    }

    // SHOULD PASS: ERC20 transfer excluded (handled by erc20-unchecked-transfer)
    function good4(address to, uint256 amt) external {
        token.transfer(to, amt);
    }

    // SHOULD PASS: ERC20 transferFrom excluded
    function good5(address from, address to, uint256 amt) external {
        token.transferFrom(from, to, amt);
    }

    // SHOULD PASS: ambiguous overload set, getPrice(address) returns uint256 but
    // getPrice(uint256) returns nothing; conservatively skip to avoid false positives
    function good6(address t) external {
        oracleOverloaded.getPrice(t);
    }

    // SHOULD FAIL: named-arg call, arity should still be 1
    function bad4(address t) external {
        oracle.getPrice({token: t}); //~WARN: Return value of an external call is not used
    }

    // SHOULD FAIL: parenthesized receiver
    function bad5(address t) external {
        (oracle).getPrice(t); //~WARN: Return value of an external call is not used
    }

    // SHOULD FAIL: parenthesized interface cast receiver
    function bad6(address oracleAddr, address t) external {
        (IOracle(oracleAddr)).getPrice(t); //~WARN: Return value of an external call is not used
    }

    // SHOULD FAIL: tuple return has an ignored slot
    function bad7(address t) external {
        (uint256 price, ) = oracle.latest(t); //~WARN: Return value of an external call is not used
        price = price + 1;
    }

    // SHOULD FAIL: tuple assignment has an ignored slot
    function bad8(address t) external {
        uint256 price;
        (price, ) = oracle.latest(t); //~WARN: Return value of an external call is not used
        price = price + 1;
    }

    // SHOULD PASS: captured return alone is considered used
    function good7(address t) external {
        uint256 price = oracle.getPrice(t);
        price = 1;
    }

    // SHOULD PASS: captured return is read before overwrite
    function good8(address t) external returns (uint256) {
        uint256 price = oracle.getPrice(t);
        uint256 out = price;
        price = 1;
        return out;
    }

    // SHOULD PASS: captured return read on a branch is still used
    function good9(address t, bool cond) external returns (uint256) {
        uint256 price = oracle.getPrice(t);
        if (cond) return price;
        return 0;
    }

    // SHOULD PASS: tuple return values are read
    function good10(address t) external returns (uint256) {
        (uint256 price, bool ok) = oracle.latest(t);
        uint256 out = price;
        bool ready = ok;
        if (ready) return out;
        return 0;
    }

    // SHOULD PASS: captured ERC20 transfer remains excluded
    function good11(address to, uint256 amt) external {
        bool ok = token.transfer(to, amt);
        ok = true;
    }
}
