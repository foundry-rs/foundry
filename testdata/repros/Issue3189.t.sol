// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3189
contract MyContract {
    function foo(uint256 arg) public returns (uint256) {
        return arg + 2;
    }
}

contract MyContractUser is DSTest {
    MyContract immutable myContract;

    constructor() {
        myContract = new MyContract();
    }

    function foo(uint256 arg) public returns (uint256 ret) {
        ret = myContract.foo(arg);
        assertEq(ret, arg + 1, "Invariant failed");
    }
}

contract Issue3189Test is DSTest {
    function testFoo() public {
        MyContractUser user = new MyContractUser();
        uint256 fooRet = user.foo(123);
    }
}
