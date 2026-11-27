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

contract A {
    struct Item {
        uint256 value;
    }

    uint256 public x;

    function setItem(Item memory item) public {
        x = item.value;
    }

    fallback() external {
        setItem(Item(1));
    }
}

contract B {
    struct Item {
        address value;
    }

    address public x;

    function setItem(Item memory item) public {
        x = item.value;
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

    function test_same_named_structs() public {
        (bool ok,) = address(new A()).call("");
        require(ok);
        new B();
    }
}
