// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";

struct FuzzTarget {
    address target;
    string abiPath;
}

abstract contract BaseHello {
    // Default false
    bool public world; // slot 0
    bool public water; // slot 1
}

contract Hello1 is BaseHello {
    function changeWorld() public {
        world = true;
    }
}

contract Hello2 is BaseHello {
    function changeWater() public {
        water = true;
    }
}

contract FullHello is Hello1, Hello2 {}

contract HelloProxy {
    address internal immutable _implementation;

    constructor(address implementation_) {
        _implementation = implementation_;
    }

    function _delegate(address implementation) internal {
        assembly {
            calldatacopy(0, 0, calldatasize())

            let result := delegatecall(gas(), implementation, 0, calldatasize(), 0, 0)

            returndatacopy(0, 0, returndatasize())

            switch result
            case 0 { revert(0, returndatasize()) }
            default { return(0, returndatasize()) }
        }
    }

    fallback() external payable {
        _delegate(_implementation);
    }
}

contract BaseTest is DSTest {
    HelloProxy proxy;

    function setUp() public {
        FullHello hello = new FullHello();
        proxy = new HelloProxy(address(hello));

        Hello1(address(hello)).changeWorld();
        Hello2(address(hello)).changeWater();
    }

    function targetProxies() public returns (FuzzTarget[] memory) {
        FuzzTarget[] memory proxies = new FuzzTarget[](2);

        proxies[0].target = address(proxy);
        proxies[0].abiPath = "Hello1";

        proxies[1].target = address(proxy);
        proxies[1].abiPath = "Hello2";

        return proxies;
    }
}

contract TargetWorldProxies is BaseTest {
    function invariantTrueWorld() public {
        require(BaseHello(address(proxy)).world() == false, "false world.");
    }
}

contract TargetWaterProxies is BaseTest {
    function invariantTrueWater() public {
        require(BaseHello(address(proxy)).water() == false, "false water.");
    }
}
