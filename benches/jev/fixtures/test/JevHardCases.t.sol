// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

struct PoolKey {
    address token0;
    address token1;
    uint128 fee;
    uint32 tickSpacing;
    address extension;
}

contract PoolIdTarget {
    bool public violated;

    function check(PoolKey memory a, PoolKey memory b) external {
        bytes32 aId = keccak256(abi.encode(a.token0, a.token1, a.fee, a.tickSpacing));
        bytes32 bId = keccak256(abi.encode(b.token0, b.token1, b.fee, b.tickSpacing));
        bool equal = a.token0 == b.token0 && a.token1 == b.token1 && a.fee == b.fee && a.tickSpacing == b.tickSpacing
            && a.extension == b.extension;
        if (equal != (aId == bId)) violated = true;
    }
}

contract JevPoolIdTest {
    PoolIdTarget private target;

    function setUp() public {
        target = new PoolIdTarget();
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(target);
    }

    function invariant_poolIdIncludesEveryField() public view {
        assert(!target.violated());
    }
}

contract RarelyFalseTarget {
    bool public violated;

    function check(uint256 n) external {
        if (n >= 1 && n <= type(uint256).max - 1234 && (n + 1234) % (1 << 80) == 0) {
            violated = true;
        }
    }
}

contract JevRarelyFalseTest {
    RarelyFalseTarget private target;

    function setUp() public {
        target = new RarelyFalseTarget();
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(target);
    }

    function invariant_modularPredicateHolds() public view {
        assert(!target.violated());
    }
}

contract SimpleStateTarget {
    uint256 public phase;

    function step1(uint256 value) external {
        if (value == 1337) phase = 1;
    }

    function step2(uint256 value) external {
        if (phase == 1 && value == 7331) phase = 2;
    }

    function step3(uint256 value) external {
        if (phase == 2 && value == 12345) phase = 3;
    }
}

contract JevSimpleStateTest {
    SimpleStateTarget private target;

    function setUp() public {
        target = new SimpleStateTarget();
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(target);
    }

    function invariant_phaseUnderThree() public view {
        assert(target.phase() < 3);
    }
}

contract ParadeLengthTarget {
    address[] private values;
    bool private checking;

    function pushOne(address value) external {
        values.push(value);
    }

    function plusFive(address value) external {
        for (uint256 i; i < 5; ++i) {
            values.push(value);
        }
    }

    function double(address value) external {
        uint256 length = values.length;
        for (uint256 i; i < length; ++i) {
            values.push(value);
        }
    }

    function popOne() external {
        if (values.length != 0) values.pop();
    }

    function enableChecking() external {
        checking = true;
    }

    function tooLong() external view returns (bool) {
        return checking && values.length >= 64;
    }
}

contract JevParadeLengthTest {
    ParadeLengthTarget private target;

    function setUp() public {
        target = new ParadeLengthTarget();
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(target);
    }

    function invariant_lengthStaysBelowLimit() public view {
        assert(!target.tooLong());
    }
}
