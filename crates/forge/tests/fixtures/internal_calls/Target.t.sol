
library L {
    function twice(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }
}

contract Target {
    uint256 public s;

    modifier m() {
        s += 1;
        _;
    }

    function c(uint256 x) internal pure returns (uint256) {
        return x + 3;
    }

    function b(uint256 x) internal pure returns (uint256) {
        return c(x) * 5;
    }

    function a(uint256 x) internal pure returns (uint256) {
        return b(x) + 7;
    }

    function runChain(uint256 x) external pure returns (uint256) {
        return a(x);
    }

    function fib(uint256 n) internal pure returns (uint256) {
        if (n < 2) return n;
        return fib(n - 1) + fib(n - 2);
    }

    function runFib(uint256 n) external pure returns (uint256) {
        return fib(n);
    }

    function step(uint256 acc, uint256 i) internal returns (uint256) {
        s += i;
        return acc * 3 + i;
    }

    function runLoop(uint256 n) external returns (uint256 acc) {
        for (uint256 i; i < n; ++i) {
            acc = step(acc, i);
        }
    }

    function t2(uint256 x) internal returns (uint256) {
        s += x;
        return x * 3;
    }

    function t1(uint256 x) internal returns (uint256) {
        s = x;
        return t2(x + 1);
    }

    function runTail(uint256 x) external returns (uint256) {
        return t1(x);
    }

    function br(uint256 x) internal returns (uint256) {
        if (x > 10) {
            s = 1;
            return x - 10;
        }
        if (x > 5) {
            s = 2;
            return x - 5;
        }
        s = 3;
        return x;
    }

    function runBranches(uint256 x) external returns (uint256) {
        return br(x) + br(x + 6) + br(x + 11);
    }

    function sq(uint256 x) internal pure returns (uint256) {
        return x * x;
    }

    function dbl(uint256 x) internal pure returns (uint256) {
        return x + x;
    }

    function runPtr(uint256 x) external pure returns (uint256) {
        function(uint256) internal pure returns (uint256) f = x % 2 == 0 ? sq : dbl;
        return f(x) + f(x + 1);
    }

    function runLib(uint256 x) external pure returns (uint256 r) {
        for (uint256 i; i < 3; ++i) {
            r += L.twice(x + i);
        }
    }

    function g(uint256 x) internal pure returns (uint256) {
        return x ^ 0x55;
    }

    function h(uint256 x) internal pure returns (uint256) {
        return x | 0x100;
    }

    function f2(uint256 x, uint256 y) internal pure returns (uint256) {
        return x * y;
    }

    function runNested(uint256 x) external pure returns (uint256) {
        return f2(g(x), h(x));
    }

    function withMod(uint256 x) internal m returns (uint256) {
        return x + s;
    }

    function runMod(uint256 x) external returns (uint256) {
        return withMod(x);
    }

    function ov(uint256 x) internal pure returns (uint256) {
        return x + 1;
    }

    function ov(uint256 x, uint256 y) internal pure returns (uint256) {
        return x + y;
    }

    function runOverload(uint256 x) external pure returns (uint256) {
        return ov(x) + ov(x, 2);
    }

    function big(uint256 x) internal returns (uint256) {
        for (uint256 i; i < x % 7 + 3; ++i) {
            s ^= x + i;
        }
        return s;
    }

    function runBig(uint256 x) external returns (uint256) {
        return big(x) + big(x + 1);
    }

    function split(uint256 x) internal pure returns (uint256 lo, uint256 hi) {
        lo = x & 0xff;
        hi = x >> 8;
    }

    function sumMem(uint256[] memory xs) internal pure returns (uint256 t) {
        for (uint256 i; i < xs.length; ++i) {
            t += xs[i];
        }
    }

    function runMulti(uint256 x) external pure returns (uint256) {
        (uint256 lo, uint256 hi) = split(x);
        uint256[] memory xs = new uint256[](3);
        xs[0] = lo;
        xs[1] = hi;
        xs[2] = x;
        return sumMem(xs);
    }
}

contract TargetTest {
    function test_chain() public {
        new Target().runChain(5);
    }

    function test_fib() public {
        new Target().runFib(6);
    }

    function test_loop() public {
        new Target().runLoop(5);
    }

    function test_tail() public {
        new Target().runTail(9);
    }

    function test_branches() public {
        new Target().runBranches(3);
    }

    function test_ptr() public {
        new Target().runPtr(4);
    }

    function test_lib() public {
        new Target().runLib(1);
    }

    function test_nested() public {
        new Target().runNested(7);
    }

    function test_mod() public {
        new Target().runMod(4);
    }

    function test_overload() public {
        new Target().runOverload(8);
    }

    function test_big() public {
        new Target().runBig(10);
    }

    function test_multi() public {
        new Target().runMulti(0x1234);
    }
}
