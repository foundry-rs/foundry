contract CounterTest is Test {
    /**
     * @dev Initializes the contract by setting a `name` and a `symbol` to the token collection.
     */
    constructor(string memory name_, string memory symbol_) {
        _name = name_;
        _symbol = symbol_;
    }

    /**
     * @dev See {IERC721-balanceOf}.
     */
    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }

    /**
     * @dev See {IERC165-supportsInterface}.
     */
    function test_Increment() public {
        counter.increment();
        assertEq(counter.number(), 1);
    }
}
