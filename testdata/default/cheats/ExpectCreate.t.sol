// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Contract {
    function add(uint256 a, uint256 b) public pure returns (uint256) {
        return a + b;
    }
}

contract EmptyCode {
    constructor() {
        assembly {
            return(0, 0)
        }
    }
}

contract ContractDeployer {
    function deployContract() public {
        new Contract();
    }

    function deployContractCreate2() public {
        new Contract{salt: "foo"}();
    }
}

contract ExpectCreateTest is Test {
    bytes bytecode = vm.getDeployedCode("cheats/ExpectCreate.t.sol:Contract");

    function testExpectCreate() public {
        vm.expectCreate(bytecode, address(this));
        new Contract();
    }

    function testExpectCreate2() public {
        vm.expectCreate2(bytecode, address(this));
        new Contract{salt: "foo"}();
    }

    function testExpectNestedCreate() public {
        ContractDeployer foo = new ContractDeployer();
        vm.expectCreate(bytecode, address(foo));
        vm.expectCreate2(bytecode, address(foo));
        foo.deployContract();
        foo.deployContractCreate2();
    }

    function testExpectCreateEmptyCode() public {
        vm.expectCreate(hex"", address(this));
        new EmptyCode();
    }
}
