// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

interface ITarget {
    event Foo(address emitter) anonymous;
}

contract Target is ITarget {
    function doEmit() external {
        emit Foo(address(this));
    }
}

// https://github.com/foundry-rs/foundry/issues/7457
contract Issue7457Test is DSTest, ITarget {
    Vm constant vm = Vm(HEVM_ADDRESS);

    Target public target;

    function setUp() external {
        target = new Target();
    }

    function testEmit0() external {
        vm.expectEmit(address(target));
        emit Foo(address(target));
        target.doEmit();
    }

    function testEmit1() external {
        vm.expectEmit(true, true, true, true, address(target));
        emit Foo(address(target));
        target.doEmit();
    }

    function testEmit2() external {
        vm.expectEmit(true, true, true, true);
        emit Foo(address(target));
        target.doEmit();
    }

    function testEmit3() external {
        vm.expectEmit(false, false, false, true);
        emit Foo(address(target));
        target.doEmit();
    }
}
