// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

interface ITarget {
    event AnonymousEventEmpty() anonymous;
    event AnonymousEventNonIndexed(uint256 a) anonymous;

    event DifferentAnonymousEventEmpty() anonymous;
    event DifferentAnonymousEventNonIndexed(string a) anonymous;

    event AnonymousEventWith1Topic(uint256 indexed a, uint256 b) anonymous;
    event AnonymousEventWith2Topics(uint256 indexed a, uint256 indexed b, uint256 c) anonymous;
    event AnonymousEventWith3Topics(uint256 indexed a, uint256 indexed b, uint256 indexed c, uint256 d) anonymous;
    event AnonymousEventWith4Topics(
        uint256 indexed a, uint256 indexed b, uint256 indexed c, uint256 indexed d, uint256 e
    ) anonymous;
}

contract Target is ITarget {
    function emitAnonymousEventEmpty() external {
        emit AnonymousEventEmpty();
    }

    function emitAnonymousEventNonIndexed(uint256 a) external {
        emit AnonymousEventNonIndexed(a);
    }

    function emitAnonymousEventWith1Topic(uint256 a, uint256 b) external {
        emit AnonymousEventWith1Topic(a, b);
    }

    function emitAnonymousEventWith2Topics(uint256 a, uint256 b, uint256 c) external {
        emit AnonymousEventWith2Topics(a, b, c);
    }

    function emitAnonymousEventWith3Topics(uint256 a, uint256 b, uint256 c, uint256 d) external {
        emit AnonymousEventWith3Topics(a, b, c, d);
    }

    function emitAnonymousEventWith4Topics(uint256 a, uint256 b, uint256 c, uint256 d, uint256 e) external {
        emit AnonymousEventWith4Topics(a, b, c, d, e);
    }
}

// https://github.com/foundry-rs/foundry/issues/7457
contract Issue7457Test is DSTest, ITarget {
    Vm constant vm = Vm(HEVM_ADDRESS);

    Target public target;

    function setUp() external {
        target = new Target();
    }

    function testEmitEvent() public {
        vm.expectEmitAnonymous(false, false, false, false, true);
        emit AnonymousEventEmpty();
        target.emitAnonymousEventEmpty();
    }

    function testFailEmitEventNonIndexed() public {
        vm.expectEmit(false, false, false, true);
        emit AnonymousEventNonIndexed(1);
        target.emitAnonymousEventNonIndexed(1);
    }

    function testEmitEventNonIndexed() public {
        vm.expectEmitAnonymous(false, false, false, false, true);
        emit AnonymousEventNonIndexed(1);
        target.emitAnonymousEventNonIndexed(1);
    }

    // function testFailEmitDifferentEvent() public {
    //     vm.expectEmitAnonymous(false, false, false, true);
    //     emit DifferentAnonymousEventEmpty();
    //     target.emitAnonymousEventEmpty();
    // }

    function testFailEmitDifferentEventNonIndexed() public {
        vm.expectEmitAnonymous(false, false, false, false, true);
        emit DifferentAnonymousEventNonIndexed("1");
        target.emitAnonymousEventNonIndexed(1);
    }

    function testEmitEventWith1Topic() public {
        vm.expectEmitAnonymous(true, false, false, false, true);
        emit AnonymousEventWith1Topic(1, 2);
        target.emitAnonymousEventWith1Topic(1, 2);
    }

    function testEmitEventWith2Topics() public {
        vm.expectEmitAnonymous(true, true, false, false, true);
        emit AnonymousEventWith2Topics(1, 2, 3);
        target.emitAnonymousEventWith2Topics(1, 2, 3);
    }

    function testEmitEventWith3Topics() public {
        vm.expectEmitAnonymous(true, true, true, false, true);
        emit AnonymousEventWith3Topics(1, 2, 3, 4);
        target.emitAnonymousEventWith3Topics(1, 2, 3, 4);
    }

    function testEmitEventWith4Topics() public {
        vm.expectEmitAnonymous(true, true, true, true, true);
        emit AnonymousEventWith4Topics(1, 2, 3, 4, 5);
        target.emitAnonymousEventWith4Topics(1, 2, 3, 4, 5);
    }
}
