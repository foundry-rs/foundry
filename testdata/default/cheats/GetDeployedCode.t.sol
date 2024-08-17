// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract TestContract {}

contract GetDeployedCodeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    address public constant overrideAddress = 0x0000000000000000000000000000000000000064;

    event Payload(address sender, address target, bytes data);

    function testGetCode() public {
        bytes memory fullPath = vm.getDeployedCode("fixtures/GetCode/Override.json");
        string memory expected = string(
            bytes(
                hex"60806040526004361061001e5760003560e01c806340e04f5e14610023575b600080fd5b610036610031366004610091565b610048565b60405190815260200160405180910390f35b60007fda9986ad4da7abb3f55b2d1f2009ab6ee50c5ad054092c04464c112acc4bc1103385858560405161007f9493929190610122565b60405180910390a15060009392505050565b6000806000604084860312156100a657600080fd5b83356001600160a01b03811681146100bd57600080fd5b9250602084013567ffffffffffffffff808211156100da57600080fd5b818601915086601f8301126100ee57600080fd5b8135818111156100fd57600080fd5b87602082850101111561010f57600080fd5b6020830194508093505050509250925092565b6001600160a01b0385811682528416602082015260606040820181905281018290526000828460808401376000608084840101526080601f19601f85011683010190509594505050505056fea26469706673582212202b0ba0fa4073d6f681bb1f99f0529e44583f2dc612a629f3ff0564eaa7257d1f64736f6c63430008110033"
            )
        );
        assertEq(string(fullPath), expected, "deployed code for full path was incorrect");
    }

    // this will set the deployed bytecode of the stateless contract to the `overrideAddress` and call the function that emits an event that will be `expectEmitted`
    function testCanEtchStatelessOverride() public {
        bytes memory code = vm.getDeployedCode("fixtures/GetCode/Override.json");
        vm.etch(overrideAddress, code);
        assertEq(
            overrideAddress.code,
            hex"60806040526004361061001e5760003560e01c806340e04f5e14610023575b600080fd5b610036610031366004610091565b610048565b60405190815260200160405180910390f35b60007fda9986ad4da7abb3f55b2d1f2009ab6ee50c5ad054092c04464c112acc4bc1103385858560405161007f9493929190610122565b60405180910390a15060009392505050565b6000806000604084860312156100a657600080fd5b83356001600160a01b03811681146100bd57600080fd5b9250602084013567ffffffffffffffff808211156100da57600080fd5b818601915086601f8301126100ee57600080fd5b8135818111156100fd57600080fd5b87602082850101111561010f57600080fd5b6020830194508093505050509250925092565b6001600160a01b0385811682528416602082015260606040820181905281018290526000828460808401376000608084840101526080601f19601f85011683010190509594505050505056fea26469706673582212202b0ba0fa4073d6f681bb1f99f0529e44583f2dc612a629f3ff0564eaa7257d1f64736f6c63430008110033"
        );

        Override over = Override(overrideAddress);

        vm.expectEmit(true, false, false, true);
        emit Payload(address(this), address(0), "hello");
        over.emitPayload(address(0), "hello");
    }

    function testWithVersion() public {
        TestContract test = new TestContract();
        bytes memory code = vm.getDeployedCode("cheats/GetDeployedCode.t.sol:TestContract:0.8.18");

        assertEq(address(test).code, code);

        vm._expectCheatcodeRevert("no matching artifact found");
        vm.getDeployedCode("cheats/GetDeployedCode.t.sol:TestContract:0.8.19");
    }
}

interface Override {
    function emitPayload(address target, bytes calldata message) external payable returns (uint256);
}
