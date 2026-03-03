// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract EOA {
    function foo() external { }
}

contract Mock {
    function callEOA(address eoa) external {
        EOA(eoa).foo();
    }
}

// https://github.com/foundry-rs/foundry/issues/11344
// Bug: different revertData depending on verbosity (-v vs -vvvv)
contract Issue11344 is Test {
    function testIssue(address eoa) public {
        Mock mock = new Mock();
        vm.assume(eoa.code.length == 0);
        assumeNotPrecompile(eoa);

        vm.expectRevert(abi.encodePacked("call to non-contract address ", vm.toString(eoa)));
        mock.callEOA(eoa);
    }
}

