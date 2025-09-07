contract C is Contract {
    function f(uint256 a, ) external {}
    function f2(uint256 a, bytes32 b,) external returns (uint256,) {}

    function f3() external {
        try some.invoke() returns (uint256,uint256,) {} catch {}
    }
}
