// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3189
contract MyContract {
    function foo(uint256 arg) public returns (uint256) {
        return arg + 2;
    }
}

contract MyContractUser is Test {
    MyContract immutable myContract;

    constructor() {
        myContract = new MyContract();
    }

    function foo(uint256 arg) public returns (uint256 ret) {
        ret = myContract.foo(arg);
        assertEq(ret, arg + 1, "Invariant failed");
    }
}

contract Issue3189Test is Test {
    function testFoo() public {
        MyContractUser user = new MyContractUser();
        uint256 fooRet = user.foo(123);
    }
}
