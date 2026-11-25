struct S {
    uint256 a;
}

contract Base {
    function sup(uint256 x) public pure virtual returns (uint256) {
        return x + 7;
    }
}

contract Matrix is Base {
    uint256 public calls;

    // Recursion at the first jump, with other arguments.
    function rec(uint256 n) public pure returns (uint256) {
        return n == 0 ? 0 : 1 + rec(n - 1);
    }

    // A call to itself with the same signature and the same encoded arguments.
    function same(uint256 x) public returns (uint256) {
        if (calls++ == 0) return same(x);
        return x;
    }

    // Public overloads with the same encoded argument.
    function sw(uint256 x) external pure returns (uint256) {
        return uint256(sw(int256(x)));
    }

    function sw(int256 x) public pure returns (int256) {
        return x + 1;
    }

    function av(address a) public pure returns (uint160) {
        return av(uint160(a));
    }

    function av(uint160 a) public pure returns (uint160) {
        return a + 1;
    }

    // A public overload with other arguments.
    function dc(uint256 x) public pure returns (uint256) {
        return dc(x, 2) + 1;
    }

    function dc(uint256 x, uint256 y) public pure returns (uint256) {
        return x * y;
    }

    // Struct parameters.
    function st(S memory s) public pure returns (uint256) {
        return st(s, 1);
    }

    function st(S memory s, uint256 k) public pure returns (uint256) {
        return s.a + k;
    }

    function sc(S calldata s) external pure returns (uint256) {
        return sc(s.a);
    }

    function sc(uint256 x) public pure returns (uint256) {
        return x + 1;
    }

    // Array parameters, in calldata and in memory.
    function ar(uint256[] calldata xs) external pure returns (uint256) {
        return ar(xs, 1);
    }

    function ar(uint256[] memory xs, uint256 k) internal pure returns (uint256) {
        return xs.length + k + 100;
    }

    function am(uint256[] memory xs) public pure returns (uint256) {
        return am(xs, 0);
    }

    function am(uint256[] memory xs, uint256 k) public pure returns (uint256) {
        return xs.length + k;
    }

    // Internal overloads.
    function io(uint256 x) public pure returns (uint256) {
        return io(x, 5);
    }

    function io(uint256 x, uint256 y) internal pure returns (uint256) {
        return x + y;
    }

    function ios(uint256 x) external pure returns (uint256) {
        return uint256(ios(int256(x)));
    }

    function ios(int256 x) internal pure returns (int256) {
        return x * 2;
    }

    // An inherited public function that calls its base.
    function sup(uint256 x) public pure override returns (uint256) {
        return super.sup(x) * 2;
    }

    // A function that the call reaches externally and internally.
    function both(uint256 x) public returns (uint256 r) {
        if (x == 0) return 1;
        r = both(x - 1);
        if (x == 2) r += this.both(1);
    }
}

contract MatrixTest {
    function test_rec() public {
        new Matrix().rec(2);
    }

    function test_same() public {
        new Matrix().same(7);
    }

    function test_sw() public {
        new Matrix().sw(uint256(3));
    }

    function test_av() public {
        new Matrix().av(address(0x1234));
    }

    function test_dc() public {
        new Matrix().dc(4);
    }

    function test_st() public {
        new Matrix().st(S(4));
    }

    function test_sc() public {
        new Matrix().sc(S(4));
    }

    function test_ar() public {
        uint256[] memory xs = new uint256[](3);
        xs[0] = 1;
        xs[1] = 2;
        xs[2] = 3;
        new Matrix().ar(xs);
    }

    function test_am() public {
        uint256[] memory xs = new uint256[](2);
        xs[0] = 1;
        xs[1] = 2;
        new Matrix().am(xs);
    }

    function test_io() public {
        new Matrix().io(2);
    }

    function test_ios() public {
        new Matrix().ios(uint256(3));
    }

    function test_sup() public {
        new Matrix().sup(5);
    }

    function test_both() public {
        new Matrix().both(2);
    }
}
