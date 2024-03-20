// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

interface ITarget {
    event Event0Empty();
    event Event0WithData(uint256 a);

    event Event1(uint256 indexed a);
    event Event2(uint256 indexed a, uint256 indexed b);
    event Event3(uint256 indexed a, uint256 indexed b, uint256 indexed c);

    event AnonymousEvent0Empty() anonymous;
    event AnonymousEvent0WithData(uint256 a) anonymous;

    event AnonymousEvent1(uint256 indexed a) anonymous;
    event AnonymousEvent2(uint256 indexed a, uint256 indexed b) anonymous;
    event AnonymousEvent3(uint256 indexed a, uint256 indexed b, uint256 indexed c) anonymous;
    event AnonymousEvent4(uint256 indexed a, uint256 indexed b, uint256 indexed c, uint256 indexed d) anonymous;
}

contract Target is ITarget {
    function emitEvent0Empty() external {
        emit Event0Empty();
    }

    function emitEvent0WithData(uint256 a) external {
        emit Event0WithData(a);
    }

    function emitEvent1(uint256 a) external {
        emit Event1(a);
    }

    function emitEvent2(uint256 a, uint256 b) external {
        emit Event2(a, b);
    }

    function emitEvent3(uint256 a, uint256 b, uint256 c) external {
        emit Event3(a, b, c);
    }

    function emitAnonymousEvent0Empty() external {
        emit AnonymousEvent0Empty();
    }

    function emitAnonymousEvent0WithData(uint256 a) external {
        emit AnonymousEvent0WithData(a);
    }

    function emitAnonymousEvent1(uint256 a) external {
        emit AnonymousEvent1(a);
    }

    function emitAnonymousEvent2(uint256 a, uint256 b) external {
        emit AnonymousEvent2(a, b);
    }

    function emitAnonymousEvent3(uint256 a, uint256 b, uint256 c) external {
        emit AnonymousEvent3(a, b, c);
    }

    function emitAnonymousEvent4(uint256 a, uint256 b, uint256 c, uint256 d) external {
        emit AnonymousEvent4(a, b, c, d);
    }
}

// https://github.com/foundry-rs/foundry/issues/7457
contract Issue7457Test is DSTest, ITarget {
    Vm constant vm = Vm(HEVM_ADDRESS);

    Target public target;

    function setUp() external {
        target = new Target();
    }

    function testEmitEvent0() public {
        vm.expectEmit(false, false, false, true);
        emit Event0Empty();
        target.emitEvent0Empty();

        vm.expectEmit(false, false, false, true);
        emit AnonymousEvent0Empty();
        target.emitAnonymousEvent0Empty();

        vm.expectEmit(false, false, false, true);
        emit Event0WithData(1);
        target.emitEvent0WithData(1);

        vm.expectEmit(false, false, false, true);
        emit AnonymousEvent0WithData(1);
        target.emitAnonymousEvent0WithData(1);
    }

    function testEmitEvent1() public {
        vm.expectEmit(true, false, false, true);
        emit Event1(1);
        target.emitEvent1(1);

        vm.expectEmit(true, false, false, true);
        emit AnonymousEvent1(1);
        target.emitAnonymousEvent1(1);
    }

    function testEmitEvent2() public {
        vm.expectEmit(true, true, false, true);
        emit Event2(1, 2);
        target.emitEvent2(1, 2);

        vm.expectEmit(true, true, false, true);
        emit AnonymousEvent2(1, 2);
        target.emitAnonymousEvent2(1, 2);
    }

    function testEmitEvent3() public {
        vm.expectEmit(true, true, true, true);
        emit Event3(1, 2, 3);
        target.emitEvent3(1, 2, 3);

        vm.expectEmit(true, true, true, true);
        emit AnonymousEvent3(1, 2, 3);
        target.emitAnonymousEvent3(1, 2, 3);
    }

    function testEmitEvent4() public {
        vm.expectEmit(true, true, true, true);
        emit AnonymousEvent4(1, 2, 3, 4);
        target.emitAnonymousEvent4(1, 2, 3, 4);
    }
}
