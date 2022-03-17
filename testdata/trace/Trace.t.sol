pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

contract RecursiveCall {
    TraceTest factory;

    event Depth(uint256 depth);
    event ChildDepth(uint256 childDepth);
    event CreatedChild(uint256 childDepth);

    constructor(address _factory) {
        factory = TraceTest(_factory);
    }

    function recurseCall(uint256 neededDepth, uint256 depth) public returns (uint256) {
        if (depth == neededDepth) {
            this.negativeNum();
            return neededDepth;
        }

        uint256 childDepth = this.recurseCall(neededDepth, depth + 1);
        emit ChildDepth(childDepth);

        this.someCall();
        emit Depth(depth);

        return depth;
    }

    function recurseCreate(uint256 neededDepth, uint256 depth) public returns (uint256) {
        if (depth == neededDepth) {
            return neededDepth;
        }

        RecursiveCall child = factory.create();
        emit CreatedChild(depth + 1);

        uint256 childDepth = child.recurseCreate(neededDepth, depth + 1);
        emit ChildDepth(childDepth);
        emit Depth(depth);

        return depth;
    }

    function someCall() public pure {}

    function negativeNum() public pure returns (int256) {
        return -1000000000;
    }
}

contract TraceTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    uint256 nodeId = 0;
    RecursiveCall first;

    function setUp() public {
        first = this.create();
    }

    function create() public returns (RecursiveCall) {
        RecursiveCall node = new RecursiveCall(address(this));
        cheats.label(
            address(node),
            string(abi.encodePacked("Node ", uintToString(nodeId++)))
        );

        return node;
    }

    function testRecurseCall() public {
        first.recurseCall(8, 0);
    }

    function testRecurseCreate() public {
        first.recurseCreate(8, 0);
    }
}

function uintToString(uint256 value) pure returns (string memory) {
    // Taken from OpenZeppelin
    if (value == 0) {
        return "0";
    }
    uint256 temp = value;
    uint256 digits;
    while (temp != 0) {
        digits++;
        temp /= 10;
    }
    bytes memory buffer = new bytes(digits);
    while (value != 0) {
        digits -= 1;
        buffer[digits] = bytes1(uint8(48 + uint256(value % 10)));
        value /= 10;
    }
    return string(buffer);
}
