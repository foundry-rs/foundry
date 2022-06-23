interface Unknown {
    function empty() external;
    function lookup() external returns (uint256);
}

contract TryStatement {
    Unknown unknown;

    function test() external {
        try unknown.empty() {} catch {}

        try unknown.lookup() returns (uint256) {} catch Error(string memory) {}
    }
}