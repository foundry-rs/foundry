struct Item {
    uint256 id;
}

contract Params {
    function total(Item[] calldata items, uint256 tag) internal pure returns (uint256 sum) {
        for (uint256 i; i < items.length; ++i) {
            sum += items[i].id;
        }
        return sum + tag;
    }

    function runItems(Item[] calldata items) external pure returns (uint256) {
        return total(items, 5);
    }

    function size(bytes calldata data, string calldata name, uint256 n) internal pure returns (uint256) {
        return data.length + bytes(name).length + n;
    }

    function runBytes(bytes calldata data, string calldata name) external pure returns (uint256) {
        return size(data, name, 7);
    }

    function twice(uint256 x) external pure returns (uint256) {
        return x * 2;
    }

    function invoke(function(uint256) external pure returns (uint256) f, uint256 x) internal pure returns (uint256) {
        return f(x);
    }

    function runFunction(uint256 x) external view returns (uint256) {
        return invoke(this.twice, x);
    }
}

contract ParamsTest {
    function test_calldata_struct_array() public {
        Item[] memory items = new Item[](2);
        items[0] = Item(3);
        items[1] = Item(4);
        new Params().runItems(items);
    }

    function test_calldata_bytes_and_string() public {
        new Params().runBytes("ab", "cde");
    }

    function test_external_function_type() public {
        new Params().runFunction(21);
    }
}
