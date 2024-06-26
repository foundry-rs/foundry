// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract TestContract {}

contract TestContractWithArgs {
    uint256 public a;
    uint256 public b;

    constructor(uint256 _a, uint256 _b) {
        a = _a;
        b = _b;
    }
}

contract DeployCodeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

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
}
