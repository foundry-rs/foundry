// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract A {
    event Event1();
    event Event2();

    function foo() public {
        emit Event1();
    }

    function bar() public {
        emit Event2();
    }
}

contract Issue10527Test is DSTest {
    event Event1();
    event Event2();

    Vm constant vm = Vm(HEVM_ADDRESS);

    A a;

    function setUp() public {
        a = new A();
    }

    function test_foo_Event1() public {
        vm.expectEmit(address(a));
        emit Event1();

        a.foo();
    }

    function test_foo_Event2() public {
        vm.expectEmit({emitter: address(a), count: 0});
        emit Event2();

        a.foo();
    }
}
