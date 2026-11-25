contract Public {
    function fact(uint256 n) public pure returns (uint256) {
        return n == 0 ? 1 : n * fact(n - 1);
    }

    function f(uint256 x) external pure returns (uint256) {
        return f(x, 2) + 1;
    }

    function f(uint256 x, uint256 y) public pure returns (uint256) {
        return x * y;
    }
}

contract PublicTest {
    function test_recursive_public_function() public {
        new Public().fact(3);
    }

    function test_public_overload() public {
        new Public().f(4);
    }
}
