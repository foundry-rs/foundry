// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Mock {
    uint256 private counter;

    function setCounter(uint256 _counter) external {
        counter = _counter;
    }
}

contract DelegateProxy {
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

contract Issue9643Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_storage_json_diff() public {
        vm.startStateDiffRecording();
        Mock proxied = Mock(address(new DelegateProxy(address(new Mock()))));
        proxied.setCounter(42);
        string memory rawDiff = vm.getStateDiffJson();
        assertEq(
            "{\"0x185a4dc360ce69bdccee33b3784b0282f7961aea\":{\"label\":null,\"balanceDiff\":null,\"nonceDiff\":{\"previousValue\":0,\"newValue\":1},\"stateDiff\":{\"0x0000000000000000000000000000000000000000000000000000000000000000\":{\"previousValue\":\"0x0000000000000000000000000000000000000000000000000000000000000000\",\"newValue\":\"0x000000000000000000000000000000000000000000000000000000000000002a\"}}},\"0xce71065d4017f316ec606fe4422e11eb2c47c246\":{\"label\":null,\"balanceDiff\":null,\"nonceDiff\":{\"previousValue\":0,\"newValue\":1},\"stateDiff\":{}}}",
            rawDiff
        );
    }
}
