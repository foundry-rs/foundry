pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract DebugLogsTest is DSTest {
    constructor() {
        emit log_uint(0);
    }

    function setUp() public {
        emit log_uint(1);
    }

    function test1() public {
        emit log_uint(2);
    }

    function test2() public {
        emit log_uint(3);
    }

    function testFailWithRevert() public {
        Fails fails = new Fails();
        emit log_uint(4);
        fails.failure();
    }

    function testFailWithRequire() public {
        emit log_uint(5);
        require(false);
    }
}

contract Fails is DSTest {
    function failure() public {
        emit log_uint(100);
        revert();
    }
}
