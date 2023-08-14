// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";

struct FuzzInterface {
    address target;
    string[] artifacts;
}

contract Hello {
    bool public world; 

    function changeWorld() external {
        world = true;
    }
}

interface IHello {
    function world() external view returns (bool);
    function changeWorld() external;
}

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

contract TargetWorldInterfaces is DSTest {
    IHello proxy;

    function setUp() public {
        Hello hello = new Hello();
        proxy = IHello(address(new HelloProxy(address(hello))));
    }

    function targetInterfaces() public returns (FuzzInterface[] memory) {
        FuzzInterface[] memory targets = new FuzzInterface[](1);

        string[] memory artifacts = new string[](1);
        artifacts[0] = "IHello";

        targets[0] = FuzzInterface(address(proxy), artifacts);

        return targets;
    }

    function invariantTrueWorld() public {
        require(proxy.world() == false, "false world.");
    }
}
