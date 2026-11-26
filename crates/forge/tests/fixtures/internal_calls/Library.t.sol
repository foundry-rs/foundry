type Signed is int256;

library Lib {
    enum K {
        A,
        B
    }

    struct D {
        uint256 v;
    }

    struct S {
        uint256 v;
    }

    struct T {
        uint256 v;
    }

    function kind(K k, uint256 x) public pure returns (uint256) {
        return uint256(uint8(k)) + x;
    }

    function set(D storage d, uint256 x) public {
        d.v = x;
    }

    function plain(uint256 x) public pure returns (uint256) {
        return x + 1;
    }

    function g(S memory s) public pure returns (uint256) {
        return g(T(s.v));
    }

    function g(T memory t) public pure returns (uint256) {
        return t.v == 0 ? 1 : g(T(t.v - 1)) + 1;
    }

    function f(uint256 x) public pure returns (uint256) {
        return uint256(Signed.unwrap(f(Signed.wrap(int256(x)))));
    }

    function f(Signed x) public pure returns (Signed) {
        int256 n = Signed.unwrap(x);
        return n <= 0 ? Signed.wrap(1) : Signed.wrap(Signed.unwrap(f(Signed.wrap(n - 1))) + 1);
    }
}

contract User {
    Lib.D d;

    function run() external returns (uint256) {
        Lib.set(d, 5);
        return Lib.kind(Lib.K.B, 5) + Lib.plain(7) + Lib.f(uint256(3)) + Lib.g(Lib.S(3)) + d.v;
    }
}

contract LibraryTest {
    function test_library_functions() public {
        new User().run();
    }
}
