library Lib {
    enum K {
        A,
        B
    }

    struct D {
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
}

contract User {
    Lib.D d;

    function run() external returns (uint256) {
        Lib.set(d, 5);
        return Lib.kind(Lib.K.B, 5) + Lib.plain(7) + d.v;
    }
}

contract LibraryTest {
    function test_library_functions() public {
        new User().run();
    }
}
