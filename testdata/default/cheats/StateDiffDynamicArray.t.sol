// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.35;

import "utils/Test.sol";

contract DynamicArrays {
    struct Item {
        uint256 a;
        address b;
    }

    struct Holder {
        uint256 count;
        address[] list;
    }

    uint256[] public nums;
    uint128[] public packed;
    Item[] public items;
    Holder internal holder;
    mapping(address => uint256[]) public byOwner;
    uint256[][] public nested;
    Item[][] internal grid;

    function pushNums() public {
        nums.push(7);
        nums.push(9);
    }

    function pushPacked() public {
        packed.push(5);
        packed.push(6);
    }

    function pushItems() public {
        items.push(Item(1, address(0x1234)));
        items.push(Item(2, address(0x5678)));
    }

    function pushHolder() public {
        holder.count = 1;
        holder.list.push(address(0x1234));
    }

    function pushByOwner(address owner, uint256 value) public {
        byOwner[owner].push(value);
    }

    function pushNested() public {
        nested.push();
        nested[0].push(5);
    }

    function pushGrid() public {
        grid.push();
        grid[grid.length - 1].push(Item(3, address(0x9abc)));
    }
}

// OpenZeppelin's `EnumerableSet.AddressSet`, kept in an ERC-7201 namespace.
contract NamespacedSet {
    struct Set {
        bytes32[] _values;
        mapping(bytes32 => uint256) _positions;
    }

    struct AddressSet {
        Set _inner;
    }

    /// @custom:storage-location erc7201:example.storage.Registry
    struct RegistryStorage {
        AddressSet set;
    }

    function _registry() internal pure returns (RegistryStorage storage $) {
        bytes32 slot =
            keccak256(abi.encode(uint256(keccak256("example.storage.Registry")) - 1)) & ~bytes32(uint256(0xff));
        assembly {
            $.slot := slot
        }
    }

    function add(address value) public {
        Set storage inner = _registry().set._inner;
        inner._values.push(bytes32(uint256(uint160(value))));
        inner._positions[bytes32(uint256(uint160(value)))] = inner._values.length;
    }
}

contract StateDiffDynamicArrayTest is Test {
    DynamicArrays internal arrays;
    NamespacedSet internal set;

    function setUp() public {
        arrays = new DynamicArrays();
        set = new NamespacedSet();
    }

    function testDynamicArrayElements() public {
        vm.startStateDiffRecording();
        arrays.pushNums();
        string memory json = vm.getStateDiffJson();
        string memory text = vm.getStateDiff();

        assertTrue(vm.contains(json, '"label":"nums[0]","type":"uint256","offset":0'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"7"}'));
        assertTrue(vm.contains(json, '"label":"nums[1]","type":"uint256","offset":0'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"9"}'));
        assertTrue(vm.contains(text, unicode"(nums[1], uint256): 0 → 9"));
    }

    function testPackedDynamicArrayElements() public {
        vm.startStateDiffRecording();
        arrays.pushPacked();
        string memory json = vm.getStateDiffJson();

        assertTrue(vm.contains(json, '"label":"packed[0], packed[1]","type":"(uint128,uint128)"'));
        assertTrue(vm.contains(json, '"label":"packed[0]","type":"uint128","offset":0'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"5"}'));
        assertTrue(vm.contains(json, '"label":"packed[1]","type":"uint128","offset":16'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"6"}'));
    }

    function testStructArrayElements() public {
        vm.startStateDiffRecording();
        arrays.pushItems();
        string memory json = vm.getStateDiffJson();

        assertTrue(vm.contains(json, '"label":"items[1].a","type":"uint256","offset":0'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"2"}'));
        assertTrue(vm.contains(json, '"label":"items[1].b","type":"address","offset":0'));
        assertTrue(
            vm.contains(
                json,
                '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000005678"}'
            )
        );
    }

    function testDynamicArrayInStruct() public {
        vm.startStateDiffRecording();
        arrays.pushHolder();
        string memory json = vm.getStateDiffJson();

        assertTrue(vm.contains(json, '"label":"holder.list[0]","type":"address","offset":0'));
        assertTrue(
            vm.contains(
                json,
                '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000001234"}'
            )
        );
    }

    function testDynamicArrayInMapping() public {
        vm.startStateDiffRecording();
        arrays.pushByOwner(address(0xAA), 42);
        string memory json = vm.getStateDiffJson();

        assertTrue(
            vm.contains(json, '"label":"byOwner[0x00000000000000000000000000000000000000AA][0]","type":"uint256"')
        );
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"42"}'));
    }

    function testNestedDynamicArray() public {
        vm.startStateDiffRecording();
        arrays.pushNested();
        string memory json = vm.getStateDiffJson();

        assertTrue(vm.contains(json, '"label":"nested[0][0]","type":"uint256","offset":0'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"5"}'));
    }

    function testStructArrayInDynamicArray() public {
        vm.startStateDiffRecording();
        arrays.pushGrid();
        string memory json = vm.getStateDiffJson();

        assertTrue(
            vm.contains(
                json,
                '"label":"grid[0]","type":"struct DynamicArrays.Item[]","offset":0,"slot":"75276140696391174450305814049576319106646922510300487059720162673006384432776","decoded":{"previousValue":"0","newValue":"1"}'
            )
        );
        assertTrue(vm.contains(json, '"label":"grid[0][0].a","type":"uint256","offset":0'));
        assertTrue(vm.contains(json, '"decoded":{"previousValue":"0","newValue":"3"}'));
        assertTrue(vm.contains(json, '"label":"grid[0][0].b","type":"address","offset":0'));
        assertTrue(
            vm.contains(
                json,
                '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000009ABc"}'
            )
        );
    }

    function testNamespacedEnumerableSet() public {
        vm.startStateDiffRecording();
        set.add(address(0x1234));
        string memory json = vm.getStateDiffJson();

        assertTrue(
            vm.contains(json, '"label":"example.storage.Registry.set._inner._values[0]","type":"bytes32","offset":0')
        );
        assertTrue(
            vm.contains(
                json,
                '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000000000000000000000000000","newValue":"0x0000000000000000000000000000000000000000000000000000000000001234"}'
            )
        );
    }
}
