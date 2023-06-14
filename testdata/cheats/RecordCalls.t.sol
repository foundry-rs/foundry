// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract RecordCallsTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testRecordCalls() public {
        cheats.recordCalls();
        address(1234).call("");
        address(5678).call("");
        address(123469).call("");
        address[] memory called = cheats.getRecordedCalls();
        // assertEq(called.length, 2);
        // assertEq(called[0], address(1234));
        // assertEq(called[1], address(5678));

        bytes memory x;
        assembly {
            let freePtr := mload(0x40)
            x := freePtr
            mstore(0x40, add(0x20, returndatasize()))
            mstore(freePtr, returndatasize())
            returndatacopy(add(freePtr, 0x20), 0x0, returndatasize())
        }
        emit log_bytes(x);
    }
}
