// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Mock {
    uint256 private counter;

    function setCounter(uint256 _counter) external {
        counter = _counter;
    }
}

contract DelegateProxy {
    // Mirrors `Mock`'s storage layout so delegated writes decode the same with either layout.
    uint256 private counter;
    address internal implementation;

    constructor(address mock) {
        implementation = mock;
    }

    fallback() external payable {
        address addr = implementation;

        assembly {
            calldatacopy(0, 0, calldatasize())
            let result := delegatecall(gas(), addr, 0, calldatasize(), 0, 0)
            returndatacopy(0, 0, returndatasize())
            switch result
            case 0 { revert(0, returndatasize()) }
            default { return(0, returndatasize()) }
        }
    }
}

contract Issue9643Test is Test {
    function test_storage_json_diff() public {
        vm.startStateDiffRecording();
        Mock proxied = Mock(address(new DelegateProxy(address(new Mock()))));
        proxied.setCounter(42);
        string memory rawDiff = vm.getStateDiffJson();
        assertEq(
            '{"0x2e234dae75c793f67a35089c9d99245e1c58470b":{"label":null,"contract":"default/repros/Issue9643.t.sol:DelegateProxy","balanceDiff":null,"nonceDiff":{"previousValue":0,"newValue":1},"stateDiff":{"0x0000000000000000000000000000000000000000000000000000000000000000":{"previousValue":"0x0000000000000000000000000000000000000000000000000000000000000000","newValue":"0x000000000000000000000000000000000000000000000000000000000000002a","label":"counter","type":"uint256","offset":0,"slot":"0","decoded":{"previousValue":"0","newValue":"42"}},"0x0000000000000000000000000000000000000000000000000000000000000001":{"previousValue":"0x0000000000000000000000000000000000000000000000000000000000000000","newValue":"0x0000000000000000000000005615deb798bb3e4dfa0139dfa1b3d433cc23b72f","label":"implementation","type":"address","offset":0,"slot":"1","decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f"}}}},"0x5615deb798bb3e4dfa0139dfa1b3d433cc23b72f":{"label":null,"contract":"default/repros/Issue9643.t.sol:Mock","balanceDiff":null,"nonceDiff":{"previousValue":0,"newValue":1},"stateDiff":{}}}',
            rawDiff
        );
    }
}
