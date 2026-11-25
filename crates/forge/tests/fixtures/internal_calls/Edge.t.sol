struct Item {
    uint256 id;
}

contract Callee {
    function ping() external pure returns (uint256) {
        return 1;
    }
}

contract Edge {
    uint256 public x;
    Callee callee = new Callee();

    function setItem(Item memory item) public {
        x = item.id;
    }

    fallback() external {
        setItem(Item(7));
    }

    function ends() internal {
        Callee c = callee;
        assembly {
            mstore(0, shl(224, 0x5c36b186))
            pop(call(gas(), c, 0, 0, 4, 0, 0))
            stop()
        }
    }

    function runEnds() external {
        ends();
    }
}

contract EdgeTest {
    function test_ends_in_a_function() public {
        new Edge().runEnds();
    }

    function test_fallback_calls_a_public_function() public {
        (bool ok,) = address(new Edge()).call("");
        require(ok);
    }

    function test_struct_entry_is_the_external_call() public {
        new Edge().setItem(Item(5));
    }
}
