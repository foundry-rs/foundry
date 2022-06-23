contract Unknown {
    function lookup() public returns(uint256) {}
}

contract TryStatement {
    Unknown unknown;

    function test() external {
        try unknown.lookup() returns (uint256) {} catch Error(string memory) {}
    }
}