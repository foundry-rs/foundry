interface Interface {
    event Bar(address indexed x);
    event Foo(address x);

    function guess(uint8 n, address x) payable external;
    function isComplete() view external returns (bool example, string memory);
}
