// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract TestContract {}

contract TestContractWithArgs {
    uint256 public a;
    uint256 public b;

    constructor(uint256 _a, uint256 _b) {
        a = _a;
        b = _b;
    }
}

contract TestPayableContract {
    uint256 public a;

    constructor() payable {
        a = msg.value;
    }
}

contract TestPayableContractWithArgs {
    uint256 public a;
    uint256 public b;
    uint256 public c;

    constructor(uint256 _a, uint256 _b) payable {
        a = _a;
        b = _b;
        c = msg.value;
    }
}

contract DeployCodeTest is Test {
    address public constant overrideAddress = 0x0000000000000000000000000000000000000064;

    event Payload(address sender, address target, bytes data);

    function testDeployCode() public {
        address addrDefault = address(new TestContract());
        address addrDeployCode = vm.deployCode("cheats/DeployCode.t.sol:TestContract");

        assertEq(addrDefault.code, addrDeployCode.code);
    }

    function testDeployCodeWithArgs() public {
        address withNew = address(new TestContractWithArgs(1, 2));
        TestContractWithArgs withDeployCode =
            TestContractWithArgs(vm.deployCode("cheats/DeployCode.t.sol:TestContractWithArgs", abi.encode(3, 4)));

        assertEq(withNew.code, address(withDeployCode).code);
        assertEq(withDeployCode.a(), 3);
        assertEq(withDeployCode.b(), 4);
    }

    function testDeployCodeWithPayableConstructorAndArgs() public {
        address withNew = address(new TestPayableContractWithArgs(1, 2));
        TestPayableContractWithArgs withDeployCode = TestPayableContractWithArgs(
            vm.deployCode("cheats/DeployCode.t.sol:TestPayableContractWithArgs", abi.encode(3, 4), 101)
        );

        assertEq(withNew.code, address(withDeployCode).code);
        assertEq(withDeployCode.a(), 3);
        assertEq(withDeployCode.b(), 4);
        assertEq(withDeployCode.c(), 101);
    }

    function testDeployCodeWithPayableConstructor() public {
        address withNew = address(new TestPayableContract());
        TestPayableContract withDeployCode =
            TestPayableContract(vm.deployCode("cheats/DeployCode.t.sol:TestPayableContract", 111));

        assertEq(withNew.code, address(withDeployCode).code);
        assertEq(withDeployCode.a(), 111);
    }

    function testDeployCodeWithSalt() public {
        address addrDefault = address(new TestContract());
        address addrDeployCode = vm.deployCode("cheats/DeployCode.t.sol:TestContract", bytes32("salt"));

        assertEq(addrDefault.code, addrDeployCode.code);
    }

    function testDeployCodeWithArgsAndSalt() public {
        address withNew = address(new TestContractWithArgs(1, 2));
        TestContractWithArgs withDeployCode = TestContractWithArgs(
            vm.deployCode("cheats/DeployCode.t.sol:TestContractWithArgs", abi.encode(3, 4), bytes32("salt"))
        );

        assertEq(withNew.code, address(withDeployCode).code);
        assertEq(withDeployCode.a(), 3);
        assertEq(withDeployCode.b(), 4);
    }

    function testDeployCodeWithPayableConstructorAndSalt() public {
        address withNew = address(new TestPayableContract());
        TestPayableContract withDeployCode =
            TestPayableContract(vm.deployCode("cheats/DeployCode.t.sol:TestPayableContract", 111, bytes32("salt")));

        assertEq(withNew.code, address(withDeployCode).code);
        assertEq(withDeployCode.a(), 111);
    }

    function testDeployCodeWithPayableConstructorAndArgsAndSalt() public {
        address withNew = address(new TestPayableContractWithArgs(1, 2));
        TestPayableContractWithArgs withDeployCode = TestPayableContractWithArgs(
            vm.deployCode("cheats/DeployCode.t.sol:TestPayableContractWithArgs", abi.encode(3, 4), 101, bytes32("salt"))
        );

        assertEq(withNew.code, address(withDeployCode).code);
        assertEq(withDeployCode.a(), 3);
        assertEq(withDeployCode.b(), 4);
        assertEq(withDeployCode.c(), 101);
    }
}
