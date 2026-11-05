//@compile-flags: --only-lint reentrancy-balance

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Each helper calls the next one twice with another local in scope. The callee cannot see the
// caller's locals, so both calls must share one cached analysis instead of doubling per level.
contract ReentrancyBalanceCallFanout {
    uint256 total;

    function entry() external {
        uint256 balance = address(this).balance;
        f0(1);
        total = balance;
    }

    function f0(uint256 v) internal {
        uint256 a = v + 1;
        f1(a);
        uint256 b = a + 1;
        f1(b);
    }

    function f1(uint256 v) internal {
        uint256 a = v + 1;
        f2(a);
        uint256 b = a + 1;
        f2(b);
    }

    function f2(uint256 v) internal {
        uint256 a = v + 1;
        f3(a);
        uint256 b = a + 1;
        f3(b);
    }

    function f3(uint256 v) internal {
        uint256 a = v + 1;
        f4(a);
        uint256 b = a + 1;
        f4(b);
    }

    function f4(uint256 v) internal {
        uint256 a = v + 1;
        f5(a);
        uint256 b = a + 1;
        f5(b);
    }

    function f5(uint256 v) internal {
        uint256 a = v + 1;
        f6(a);
        uint256 b = a + 1;
        f6(b);
    }

    function f6(uint256 v) internal {
        uint256 a = v + 1;
        f7(a);
        uint256 b = a + 1;
        f7(b);
    }

    function f7(uint256 v) internal {
        uint256 a = v + 1;
        f8(a);
        uint256 b = a + 1;
        f8(b);
    }

    function f8(uint256 v) internal {
        uint256 a = v + 1;
        f9(a);
        uint256 b = a + 1;
        f9(b);
    }

    function f9(uint256 v) internal {
        uint256 a = v + 1;
        f10(a);
        uint256 b = a + 1;
        f10(b);
    }

    function f10(uint256 v) internal {
        uint256 a = v + 1;
        f11(a);
        uint256 b = a + 1;
        f11(b);
    }

    function f11(uint256 v) internal {
        uint256 a = v + 1;
        f12(a);
        uint256 b = a + 1;
        f12(b);
    }

    function f12(uint256 v) internal {
        uint256 a = v + 1;
        f13(a);
        uint256 b = a + 1;
        f13(b);
    }

    function f13(uint256 v) internal {
        uint256 a = v + 1;
        f14(a);
        uint256 b = a + 1;
        f14(b);
    }

    function f14(uint256 v) internal {
        uint256 a = v + 1;
        f15(a);
        uint256 b = a + 1;
        f15(b);
    }

    function f15(uint256 v) internal {
        uint256 a = v + 1;
        f16(a);
        uint256 b = a + 1;
        f16(b);
    }

    function f16(uint256 v) internal {
        uint256 a = v + 1;
        f17(a);
        uint256 b = a + 1;
        f17(b);
    }

    function f17(uint256 v) internal {
        uint256 a = v + 1;
        f18(a);
        uint256 b = a + 1;
        f18(b);
    }

    function f18(uint256 v) internal {
        uint256 a = v + 1;
        f19(a);
        uint256 b = a + 1;
        f19(b);
    }

    function f19(uint256 v) internal {
        uint256 a = v + 1;
        f20(a);
        uint256 b = a + 1;
        f20(b);
    }

    function f20(uint256 v) internal {
        uint256 a = v + 1;
        f21(a);
        uint256 b = a + 1;
        f21(b);
    }

    function f21(uint256 v) internal {
        uint256 a = v + 1;
        f22(a);
        uint256 b = a + 1;
        f22(b);
    }

    function f22(uint256 v) internal {
        uint256 a = v + 1;
        f23(a);
        uint256 b = a + 1;
        f23(b);
    }

    function f23(uint256 v) internal {
        uint256 a = v + 1;
        f24(a);
        uint256 b = a + 1;
        f24(b);
    }

    function f24(uint256 v) internal {
        total += v;
    }
}
