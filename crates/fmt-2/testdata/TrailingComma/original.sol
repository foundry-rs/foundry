contract C is Contract {
    modifier m(uint256, ,,, ) {}
    // invalid solidity code, but valid pt
    modifier m2(uint256) returns (uint256,,,) {}

    function f(uint256 a, ) external {}
    function f2(uint256 a, , , ,bytes32 b) external returns (uint256,,,,) {}

    function f3() external {
        try some.invoke() returns (uint256,,,uint256) {} catch {}
    }
}
