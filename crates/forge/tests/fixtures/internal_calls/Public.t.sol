struct Item {
    uint256 id;
}

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

    function h(uint256 x) external pure returns (uint256) {
        return uint256(h(int256(x)));
    }

    function h(int256 x) public pure returns (int256) {
        return x + 1;
    }

    function k(uint256 x) external pure returns (uint256) {
        return k(Item(x));
    }

    function k(Item memory item) public pure returns (uint256) {
        return item.id + 1;
    }

    function read(uint256[2][2] memory xs) public pure returns (uint256) {
        return xs[0][0] + xs[1][1];
    }
}

contract PublicTest {
    function test_recursive_public_function() public {
        new Public().fact(3);
    }

    function test_public_overload() public {
        new Public().f(4);
    }

    function test_overloads_with_the_same_argument() public {
        new Public().h(uint256(3));
    }

    function test_nested_fixed_array_argument() public {
        new Public().read([[uint256(1), 2], [uint256(3), 4]]);
    }

    function test_overloads_with_a_struct() public {
        new Public().k(uint256(3));
    }
}
