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

contract RevertingConstructor {
    constructor() {
        revert("constructor reverted");
    }
}

contract HaltingConstructor {
    constructor() {
        assembly {
            invalid()
        }
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

    function testDeployCodeWithRemapping() public {
        address addrDefault = address(new TestContract());
        address addrDeployCode = vm.deployCode("@cheats/DeployCode.t.sol:TestContract");

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

    function testDeployCodeConstructorRevert() public {
        try vm.deployCode("cheats/DeployCode.t.sol:RevertingConstructor") {
            revert("expected constructor revert");
        } catch (bytes memory reason) {
            assertEq(reason, abi.encodeWithSignature("Error(string)", "constructor reverted"));
        }
    }

    function testDeployCodeConstructorHalt() public {
        try vm.deployCode("cheats/DeployCode.t.sol:HaltingConstructor") {
            revert("expected constructor halt");
        } catch (bytes memory reason) {
            assertEq(reason, "");
        }
    }
}

/// forge-config: default.isolate = false
contract DeployCodeNonIsolatedTest is DeployCodeTest {}

/// forge-config: default.always_use_create_2_factory = true
contract DeployCodeCreate2FactoryTest is Test {
    address constant CREATE2_FACTORY = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    function testNewWithSaltUsesFactory() public {
        address deployed = address(new TestContract{salt: bytes32("salt")}());

        assertEq(
            deployed,
            vm.computeCreate2Address(bytes32("salt"), keccak256(type(TestContract).creationCode), CREATE2_FACTORY)
        );
    }

    function testNewWithSaltConstructorRevert() public {
        try new RevertingConstructor{salt: bytes32("salt")}() {
            revert("expected constructor revert");
        } catch (bytes memory reason) {
            assertEq(reason, "");
        }
    }

    function testNewWithSaltConstructorHalt() public {
        try new HaltingConstructor{salt: bytes32("salt")}() {
            revert("expected constructor halt");
        } catch (bytes memory reason) {
            assertEq(reason, "");
        }
    }
}

/// forge-config: default.always_use_create_2_factory = true
/// forge-config: default.isolate = false
contract DeployCodeCreate2FactoryNonIsolatedTest is DeployCodeCreate2FactoryTest {}
