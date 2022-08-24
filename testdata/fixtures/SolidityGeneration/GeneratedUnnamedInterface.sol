interface Interface {
    event Bar(address indexed x);
    event Foo(address x);

    function guess(uint8 n, address x) external payable;
    function isComplete() external view returns (bool example, string memory);
}
