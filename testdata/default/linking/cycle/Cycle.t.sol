// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

library Foo {
    function foo() external {
        Bar.bar();
    }

    function flum() external {}
}

library Bar {
    function bar() external {
        Foo.flum();
    }
}
