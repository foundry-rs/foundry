// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "./test.sol";
import "./Vm.sol";

contract MemSafetyFailureTest is DSTest {
    Vm constant vm = Vm(address(HEVM_ADDRESS));

    /// @dev Tests that writing to memory before the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MSTORE` opcode.
    function testShouldFailExpectSafeMemory_MSTORE_Low() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Attempt to write to memory outside of the range using `MSTORE`
        assembly {
            mstore(0x60, 0xc0ffee)
        }
    }

    /// @dev Tests that writing to memory after the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MSTORE` opcode.
    function testShouldFailExpectSafeMemory_MSTORE_High() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Attempt to write to memory outside of the range using `MSTORE`
        assembly {
            mstore(0xA0, 0xc0ffee)
        }
    }

    /// @dev Tests that writing to memory before the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MSTORE8` opcode.
    function testShouldFailExpectSafeMemory_MSTORE8_Low() public {
        // Allow memory writes in the range of [0x80, 0x81) within this context
        vm.expectSafeMemory(0x80, 0x81);

        // Attempt to write to memory outside of the range using `MSTORE8`
        assembly {
            mstore8(0x60, 0xFF)
        }
    }

    /// @dev Tests that writing to memory after the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MSTORE8` opcode.
    function testShouldFailExpectSafeMemory_MSTORE8_High() public {
        // Allow memory writes in the range of [0x80, 0x81) within this context
        vm.expectSafeMemory(0x80, 0x81);

        // Attempt to write to memory outside of the range using `MSTORE8`
        assembly {
            mstore8(0x81, 0xFF)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CALLDATACOPY` opcode.
    function testShouldFailExpectSafeMemory_CALLDATACOPY(uint256 _x) public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Write to memory outside the range using `CALLDATACOPY`
        assembly {
            calldatacopy(0xA0, 0x04, 0x20)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CODECOPY` opcode.
    function testShouldFailExpectSafeMemory_CODECOPY() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Attempt to write to memory outside of the range using `CODECOPY`
        assembly {
            let size := extcodesize(address())
            codecopy(0x80, 0x00, size)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `RETURNDATACOPY` opcode.
    function testShouldFailExpectSafeMemory_RETURNDATACOPY() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doCallReturnData(address(sc), payload, 0x80, 0x60);

        // Write to memory outside of the range using `RETURNDATACOPY`
        assembly {
            returndatacopy(0x100, 0x00, 0x60)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `EXTCODECOPY` opcode.
    function testShouldFailExpectSafeMemory_EXTCODECOPY() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Attempt to write to memory outside of the range using `EXTCODECOPY`
        assembly {
            let size := extcodesize(address())
            extcodecopy(address(), 0xA0, 0x00, 0x20)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CALL` opcode.
    function testShouldFailExpectSafeMemory_CALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doCallReturnData(address(sc), payload, 0x100, 0x60);
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CALLCODE` opcode.
    function testShouldFailExpectSafeMemory_CALLCODE() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doCallCodeReturnData(address(sc), payload, 0x100, 0x60);
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `STATICCALL` opcode.
    function testShouldFailExpectSafeMemory_STATICCALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doStaticCallReturnData(address(sc), payload, 0x100, 0x60);
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `DELEGATECALL` opcode.
    function testShouldFailExpectSafeMemory_DELEGATECALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doDelegateCallReturnData(address(sc), payload, 0x100, 0x60);
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MLOAD` opcode.
    function testShouldFailExpectSafeMemory_MLOAD() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert. Ugly hack to make sure the mload isn't optimized
        // out.
        uint256 a;
        assembly {
            a := mload(0x100)
        }
        uint256 b = a + 1;
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `SHA3` opcode.
    function testShouldFailExpectSafeMemory_SHA3() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert. Ugly hack to make sure the sha3 isn't optimized
        // out.
        uint256 a;
        assembly {
            a := keccak256(0x100, 0x20)
        }
        uint256 b = a + 1;
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `LOG0` opcode.
    function testShouldFailExpectSafeMemory_LOG0() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert.
        assembly {
            log0(0x100, 0x20)
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CREATE` opcode.
    function testShouldFailExpectSafeMemory_CREATE() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert.
        assembly {
            pop(create(0, 0x100, 0x20))
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CREATE2` opcode.
    function testShouldFailExpectSafeMemory_CREATE2() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert.
        assembly {
            pop(create2(0, 0x100, 0x20, 0x00))
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `RETURN` opcode.
    function testShouldFailExpectSafeMemory_RETURN() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert.
        assembly {
            return(0x100, 0x20)
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `REVERT` opcode.
    function testShouldFailExpectSafeMemory_REVERT() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `doRevert` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.doRevert.selector, 0x120, 0x20);

        // Expect memory in the range of [0x00, 0x120] to be safe in the next subcontext
        vm.expectSafeMemoryCall(0x00, 0x120);

        // Call `doRevert` on the SubContext contract and ensure it did not revert with
        // zero data.
        _doCallReturnData(address(sc), payload, 0x200, 0x20);
        assembly {
            if iszero(eq(keccak256(0x60, 0x20), keccak256(0x200, returndatasize()))) { revert(0x00, 0x00) }
        }
    }

    /// @dev Tests that the `expectSafeMemoryCall` cheatcode works as expected.
    function testShouldFailExpectSafeMemoryCall() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();
        // Create a payload to call `doMstore8` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.doMstore.selector, 0xA0, 0xc0ffee);

        // Allow memory writes in the range of [0x80, 0xA0) within the next created subcontext
        vm.expectSafeMemoryCall(0x80, 0xA0);

        // Should revert. The memory write in this subcontext is outside of the allowed range.
        if (!_doCall(address(sc), payload)) {
            revert("Expected call to fail");
        }
    }

    /// @dev Tests that the `stopExpectSafeMemory` cheatcode does not cause violations not being noticed.
    function testShouldFailStopExpectSafeMemory() public {
        uint64 initPtr;
        assembly {
            initPtr := mload(0x40)
        }

        vm.expectSafeMemory(initPtr, initPtr + 0x20);
        assembly {
            // write outside of allowed range, this should revert
            mstore(add(initPtr, 0x20), 0x01)
        }

        vm.stopExpectSafeMemory();
    }

    // Helpers

    /// @dev Performs a call without copying any returndata.
    function _doCall(address _target, bytes memory _payload) internal returns (bool _success) {
        assembly {
            _success := call(gas(), _target, 0x00, add(_payload, 0x20), mload(_payload), 0x00, 0x00)
        }
    }

    /// @dev Performs a call and copies returndata to memory.
    function _doCallReturnData(address _target, bytes memory _payload, uint256 returnDataDest, uint256 returnDataSize)
        internal
    {
        assembly {
            pop(call(gas(), _target, 0x00, add(_payload, 0x20), mload(_payload), returnDataDest, returnDataSize))
        }
    }

    /// @dev Performs a staticcall and copies returndata to memory.
    function _doStaticCallReturnData(
        address _target,
        bytes memory _payload,
        uint256 returnDataDest,
        uint256 returnDataSize
    ) internal {
        assembly {
            pop(staticcall(gas(), _target, add(_payload, 0x20), mload(_payload), returnDataDest, returnDataSize))
        }
    }

    /// @dev Performs a delegatecall and copies returndata to memory.
    function _doDelegateCallReturnData(
        address _target,
        bytes memory _payload,
        uint256 returnDataDest,
        uint256 returnDataSize
    ) internal {
        assembly {
            pop(delegatecall(gas(), _target, add(_payload, 0x20), mload(_payload), returnDataDest, returnDataSize))
        }
    }

    /// @dev Performs a callcode and copies returndata to memory.
    function _doCallCodeReturnData(
        address _target,
        bytes memory _payload,
        uint256 returnDataDest,
        uint256 returnDataSize
    ) internal {
        assembly {
            pop(callcode(gas(), _target, 0x00, add(_payload, 0x20), mload(_payload), returnDataDest, returnDataSize))
        }
    }
}

/// @dev A simple contract for testing the `expectSafeMemory` & `expectSafeMemoryCall` cheatcodes.
contract SubContext {
    function doMstore(uint256 offset, uint256 val) external {
        assembly {
            mstore(offset, val)
        }
    }

    function doMstore8(uint256 offset, uint8 val) external {
        assembly {
            mstore8(offset, val)
        }
    }

    function giveReturndata() external view returns (bytes memory _returndata) {
        return hex"7dc4acc68d77c9c85b5cb0f53ab9ceea175f7964390758e4409013ce80643f84";
    }

    function doRevert(uint256 offset, uint256 size) external {
        assembly {
            revert(offset, size)
        }
    }
}
