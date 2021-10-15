pragma solidity 0.8.0;

import "../../evm-adapters/testdata/DsTest.sol";

contract DebugLogsTest is DSTest {
    constructor() public {
        emit log("constructor");
    }

    function setUp() public {
        emit log("setUp");
    }

    function test1() public {
        emit log("one");
    }

    function test2() public {
        emit log("two");
    }

}
