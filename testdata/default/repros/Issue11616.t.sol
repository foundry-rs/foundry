// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.24;

import "utils/Test.sol";

contract Emit {
    event A();
    event B();

    function emitB() public {
        emit B();
    }
}

contract Issue11616Test is Test {
    Emit public e;

    function setUp() public {
        e = new Emit();
    }

    function test_emitNotOk() public {
        vm.expectEmit({count: 0});
        emit Emit.A();
        vm.expectEmit();
        emit Emit.B();
        e.emitB();
    }
}
