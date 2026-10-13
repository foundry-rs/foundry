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

    function testSaltedDeployCodeRevertCleanup() public {
        assertFailedDeployCodeCleanup("cheats/DeployCode.t.sol:RevertingConstructor");
    }

    function testSaltedDeployCodeHaltCleanup() public {
        assertFailedDeployCodeCleanup("cheats/DeployCode.t.sol:HaltingConstructor");
    }

    function testNestedDeployCodeUsesCaller() public {
        DeployCodeCaller caller = new DeployCodeCaller();
        address deployed = caller.deploy(false);
        assertEq(
            deployed,
            vm.computeCreate2Address(bytes32("nested"), keccak256(type(TestContract).creationCode), address(caller))
        );
    }

    function testNestedPrankDeployCodeUsesFactory() public {
        DeployCodeCaller caller = new DeployCodeCaller();
        (address deployed, address created) = caller.deployThenCreate();
        assertEq(
            deployed,
            vm.computeCreate2Address(bytes32("nested"), keccak256(type(TestContract).creationCode), CREATE2_FACTORY)
        );
        assertEq(
            created,
            vm.computeCreate2Address(bytes32("after"), keccak256(type(TestContract).creationCode), address(caller))
        );
    }

    function testConstructorCreatesAroundNestedDeployCode() public {
        NestedDeployCodeConstructor deployed = NestedDeployCodeConstructor(
            vm.deployCode("cheats/DeployCode.t.sol:NestedDeployCodeConstructor", bytes32("parent"))
        );
        assertEq(
            address(deployed),
            vm.computeCreate2Address(
                bytes32("parent"), keccak256(type(NestedDeployCodeConstructor).creationCode), CREATE2_FACTORY
            )
        );
        assertEq(deployed.deployer(), CREATE2_FACTORY);
        bytes32 codeHash = keccak256(type(TestContract).creationCode);
        assertEq(deployed.beforeChild(), vm.computeCreate2Address(bytes32("before"), codeHash, address(deployed)));
        assertEq(deployed.nestedChild(), vm.computeCreate2Address(bytes32("nested"), codeHash, address(deployed)));
        assertEq(deployed.afterChild(), vm.computeCreate2Address(bytes32("after"), codeHash, address(deployed)));
    }

    function assertFailedDeployCodeCleanup(string memory artifact) internal {
        address originalOrigin = tx.origin;
        vm.prank(address(1234), address(5678));
        try vm.deployCode(artifact, bytes32("failed")) {
            revert("expected constructor failure");
        } catch (bytes memory reason) {
            assertEq(reason, "");
        }
        assertEq(tx.origin, originalOrigin);
        address expected = vm.computeCreateAddress(address(this), vm.getNonce(address(this)));
        assertEq(address(new TestContract()), expected);
        DeployCodeCaller caller = new DeployCodeCaller();
        assertEq(
            caller.create(),
            vm.computeCreate2Address(bytes32("native"), keccak256(type(TestContract).creationCode), address(caller))
        );
        address deployed = vm.deployCode("cheats/DeployCode.t.sol:TestContract", bytes32("recovery"));
        assertEq(
            deployed,
            vm.computeCreate2Address(bytes32("recovery"), keccak256(type(TestContract).creationCode), CREATE2_FACTORY)
        );
    }
}

/// forge-config: default.always_use_create_2_factory = true
/// forge-config: default.isolate = false
contract DeployCodeCreate2FactoryNonIsolatedTest is DeployCodeCreate2FactoryTest {}

contract DeployCodeCaller is Test {
    function deploy(bool prank) external returns (address) {
        if (prank) vm.prank(address(1234));
        return vm.deployCode("cheats/DeployCode.t.sol:TestContract", bytes32("nested"));
    }

    function create() external returns (address) {
        return address(new TestContract{salt: bytes32("native")}());
    }

    function deployThenCreate() external returns (address deployed, address created) {
        vm.prank(address(1234));
        deployed = vm.deployCode("cheats/DeployCode.t.sol:TestContract", bytes32("nested"));
        created = address(new TestContract{salt: bytes32("after")}());
    }
}

contract NestedDeployCodeConstructor is Test {
    address public deployer;
    address public beforeChild;
    address public nestedChild;
    address public afterChild;

    constructor() {
        deployer = msg.sender;
        beforeChild = address(new TestContract{salt: bytes32("before")}());
        nestedChild = vm.deployCode("cheats/DeployCode.t.sol:TestContract", bytes32("nested"));
        afterChild = address(new TestContract{salt: bytes32("after")}());
    }
}
