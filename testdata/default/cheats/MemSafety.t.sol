// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract MemSafetyTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    ////////////////////////////////////////////////////////////////
    //                           MSTORE                           //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `MSTORE` opcode.
    function testExpectSafeMemory_MSTORE() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Write to memory within the range using `MSTORE`
        assembly {
            mstore(0x80, 0xc0ffee)
        }
    }

    /// @dev Tests that writing to memory within the ranges given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `MSTORE` opcode.
    function testExpectSafeMemory_multiRange_MSTORE() public {
        // Allow memory writes in the range of [0x80, 0x100) and [0x120, 0x140) within this context
        vm.expectSafeMemory(0x80, 0x100);
        vm.expectSafeMemory(0x120, 0x140);

        // Write to memory within the range using `MSTORE`
        assembly {
            mstore(0x80, 0xc0ffee)
            mstore(0x120, 0xbadf00d)
        }
    }

    /// @dev Tests that writing to memory before the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MSTORE` opcode.
    function testFailExpectSafeMemory_MSTORE_Low() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Attempt to write to memory outside of the range using `MSTORE`
        assembly {
            mstore(0x60, 0xc0ffee)
        }
    }

    /// @dev Tests that writing to memory after the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MSTORE` opcode.
    function testFailExpectSafeMemory_MSTORE_High() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Attempt to write to memory outside of the range using `MSTORE`
        assembly {
            mstore(0xA0, 0xc0ffee)
        }
    }

    ////////////////////////////////////////////////////////////////
    //                          MSTORE8                           //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `MSTORE8` opcode.
    function testExpectSafeMemory_MSTORE8() public {
        // Allow memory writes in the range of [0x80, 0x81) within this context
        vm.expectSafeMemory(0x80, 0x81);

        // Write to memory within the range using `MSTORE8`
        assembly {
            mstore8(0x80, 0xFF)
        }
    }

    /// @dev Tests that writing to memory within the ranges given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `MSTORE8` opcode.
    function testExpectSafeMemory_multiRange_MSTORE8() public {
        // Allow memory writes in the range of [0x80, 0x100) and [0x120, 0x121) within this context
        vm.expectSafeMemory(0x80, 0x100);
        vm.expectSafeMemory(0x120, 0x121);

        // Write to memory within the range using `MSTORE8`
        assembly {
            mstore8(0x80, 0xFF)
            mstore8(0x120, 0xFF)
        }
    }

    /// @dev Tests that writing to memory before the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MSTORE8` opcode.
    function testFailExpectSafeMemory_MSTORE8_Low() public {
        // Allow memory writes in the range of [0x80, 0x81) within this context
        vm.expectSafeMemory(0x80, 0x81);

        // Attempt to write to memory outside of the range using `MSTORE8`
        assembly {
            mstore8(0x60, 0xFF)
        }
    }

    /// @dev Tests that writing to memory after the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MSTORE8` opcode.
    function testFailExpectSafeMemory_MSTORE8_High() public {
        // Allow memory writes in the range of [0x80, 0x81) within this context
        vm.expectSafeMemory(0x80, 0x81);

        // Attempt to write to memory outside of the range using `MSTORE8`
        assembly {
            mstore8(0x81, 0xFF)
        }
    }

    ////////////////////////////////////////////////////////////////
    //                        CALLDATACOPY                        //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `CALLDATACOPY` opcode.
    function testExpectSafeMemory_CALLDATACOPY(uint256 _x) public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Write to memory within the range using `CALLDATACOPY`
        assembly {
            calldatacopy(0x80, 0x04, 0x20)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CALLDATACOPY` opcode.
    function testFailExpectSafeMemory_CALLDATACOPY(uint256 _x) public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Write to memory outside the range using `CALLDATACOPY`
        assembly {
            calldatacopy(0xA0, 0x04, 0x20)
        }
    }

    ////////////////////////////////////////////////////////////////
    //                          CODECOPY                          //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `CODECOPY` opcode.
    function testExpectSafeMemory_CODECOPY() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Write to memory within the range using `CODECOPY`
        assembly {
            let size := extcodesize(address())
            codecopy(0x80, 0x00, 0x20)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CODECOPY` opcode.
    function testFailExpectSafeMemory_CODECOPY() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Attempt to write to memory outside of the range using `CODECOPY`
        assembly {
            let size := extcodesize(address())
            codecopy(0x80, 0x00, size)
        }
    }

    ////////////////////////////////////////////////////////////////
    //                       RETURNDATACOPY                       //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `RETURNDATACOPY` opcode.
    function testExpectSafeMemory_RETURNDATACOPY() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doCallReturnData(address(sc), payload, 0x80, 0x60);

        // Write to memory within the range using `RETURNDATACOPY`
        assembly {
            returndatacopy(0x80, 0x00, 0x60)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `RETURNDATACOPY` opcode.
    function testFailExpectSafeMemory_RETURNDATACOPY() public {
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

    ////////////////////////////////////////////////////////////////
    //                        EXTCODECOPY                         //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `EXTCODECOPY` opcode.
    function testExpectSafeMemory_EXTCODECOPY() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Write to memory within the range using `EXTCODECOPY`
        assembly {
            let size := extcodesize(address())
            extcodecopy(address(), 0x80, 0x00, 0x20)
        }
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `EXTCODECOPY` opcode.
    function testFailExpectSafeMemory_EXTCODECOPY() public {
        // Allow memory writes in the range of [0x80, 0xA0) within this context
        vm.expectSafeMemory(0x80, 0xA0);

        // Attempt to write to memory outside of the range using `EXTCODECOPY`
        assembly {
            let size := extcodesize(address())
            extcodecopy(address(), 0xA0, 0x00, 0x20)
        }
    }

    ////////////////////////////////////////////////////////////////
    //                            CALL                            //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `CALL` opcode.
    function testExpectSafeMemory_CALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doCallReturnData(address(sc), payload, 0x80, 0x60);
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CALL` opcode.
    function testFailExpectSafeMemory_CALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doCallReturnData(address(sc), payload, 0x100, 0x60);
    }

    ////////////////////////////////////////////////////////////////
    //                          CALLCODE                          //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `CALLCODE` opcode.
    function testExpectSafeMemory_CALLCODE() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doCallCodeReturnData(address(sc), payload, 0x80, 0x60);
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CALLCODE` opcode.
    function testFailExpectSafeMemory_CALLCODE() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doCallCodeReturnData(address(sc), payload, 0x100, 0x60);
    }

    ////////////////////////////////////////////////////////////////
    //                         STATICCALL                         //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `STATICCALL` opcode.
    function testExpectSafeMemory_STATICCALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doStaticCallReturnData(address(sc), payload, 0x80, 0x60);
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `STATICCALL` opcode.
    function testFailExpectSafeMemory_STATICCALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doStaticCallReturnData(address(sc), payload, 0x100, 0x60);
    }

    ////////////////////////////////////////////////////////////////
    //                        DELEGATECALL                        //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that writing to memory within of the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `DELEGATECALL` opcode.
    function testExpectSafeMemory_DELEGATECALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doDelegateCallReturnData(address(sc), payload, 0x80, 0x60);
    }

    /// @dev Tests that writing to memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `DELEGATECALL` opcode.
    function testFailExpectSafeMemory_DELEGATECALL() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `giveReturndata` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.giveReturndata.selector);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Create a new SubContext contract and call `giveReturndata` on it.
        _doDelegateCallReturnData(address(sc), payload, 0x100, 0x60);
    }

    ////////////////////////////////////////////////////////////////
    //                   MLOAD (Read Expansion)                   //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that expanding memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `MLOAD` opcode.
    function testExpectSafeMemory_MLOAD() public {
        vm.expectSafeMemory(0x80, 0x120);

        // This should not revert. Ugly hack to make sure the mload isn't optimized
        // out.
        uint256 a;
        assembly {
            a := mload(0x100)
        }
        uint256 b = a + 1;
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MLOAD` opcode.
    function testExpectSafeMemory_MLOAD_REVERT() public {
        vm.expectSafeMemory(0x80, 0x100);

        vm.expectRevert();

        // This should revert. Ugly hack to make sure the mload isn't optimized
        // out.
        uint256 a;
        assembly {
            a := mload(0x100)
        }
        uint256 b = a + 1;
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `MLOAD` opcode.
    function testFailExpectSafeMemory_MLOAD() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert. Ugly hack to make sure the mload isn't optimized
        // out.
        uint256 a;
        assembly {
            a := mload(0x100)
        }
        uint256 b = a + 1;
    }

    ////////////////////////////////////////////////////////////////
    //                   SHA3 (Read Expansion)                    //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that expanding memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `SHA3` opcode.
    function testExpectSafeMemory_SHA3() public {
        vm.expectSafeMemory(0x80, 0x120);

        // This should not revert. Ugly hack to make sure the sha3 isn't optimized
        // out.
        uint256 a;
        assembly {
            a := keccak256(0x100, 0x20)
        }
        uint256 b = a + 1;
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `SHA3` opcode.
    function testFailExpectSafeMemory_SHA3() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert. Ugly hack to make sure the sha3 isn't optimized
        // out.
        uint256 a;
        assembly {
            a := keccak256(0x100, 0x20)
        }
        uint256 b = a + 1;
    }

    ////////////////////////////////////////////////////////////////
    //                 LOG(0-4) (Read Expansion)                  //
    ////////////////////////////////////////////////////////////////

    // Note: We only test LOG0 here because the other LOG opcodes have the offset
    //       and size arguments in the same position on the stack as LOG0.

    /// @dev Tests that expanding memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `LOG0` opcode.
    function testExpectSafeMemory_LOG0() public {
        vm.expectSafeMemory(0x80, 0x120);

        // This should not revert.
        assembly {
            log0(0x100, 0x20)
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `LOG0` opcode.
    function testFailExpectSafeMemory_LOG0() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert.
        assembly {
            log0(0x100, 0x20)
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `LOG0` opcode.
    function testExpectSafeMemory_LOG0_REVERT() public {
        vm.expectSafeMemory(0x80, 0x100);
        vm.expectRevert();
        // This should revert.
        assembly {
            log0(0x100, 0x20)
        }
    }

    ////////////////////////////////////////////////////////////////
    //              CREATE/CREATE2 (Read Expansion)               //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that expanding memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `CREATE` opcode.
    function testExpectSafeMemory_CREATE() public {
        vm.expectSafeMemory(0x80, 0x120);

        // This should not revert.
        assembly {
            pop(create(0, 0x100, 0x20))
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CREATE` opcode.
    function testFailExpectSafeMemory_CREATE() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert.
        assembly {
            pop(create(0, 0x100, 0x20))
        }
    }

    /// @dev Tests that expanding memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `CREATE2` opcode.
    function testExpectSafeMemory_CREATE2() public {
        vm.expectSafeMemory(0x80, 0x120);

        // This should not revert.
        assembly {
            pop(create2(0, 0x100, 0x20, 0x00))
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `CREATE2` opcode.
    function testFailExpectSafeMemory_CREATE2() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert.
        assembly {
            pop(create2(0, 0x100, 0x20, 0x00))
        }
    }

    ////////////////////////////////////////////////////////////////
    //               RETURN/REVERT (Read Expansion)               //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that expanding memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `RETURN` opcode.
    function testExpectSafeMemory_RETURN() public {
        vm.expectSafeMemory(0x80, 0x120);

        // This should not revert.
        assembly {
            return(0x100, 0x20)
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `RETURN` opcode.
    function testFailExpectSafeMemory_RETURN() public {
        vm.expectSafeMemory(0x80, 0x100);

        // This should revert.
        assembly {
            return(0x100, 0x20)
        }
    }

    /// @dev Tests that expanding memory within the range given to `expectSafeMemory`
    ///      will not cause the test to fail while using the `REVERT` opcode.
    function testExpectSafeMemory_REVERT() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();

        // Create a payload to call `doRevert` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.doRevert.selector, 0x100, 0x20);

        // Expect memory in the range of [0x00, 0x120] to be safe in the next subcontext
        vm.expectSafeMemoryCall(0x00, 0x120);

        // Call `doRevert` on the SubContext contract and ensure it did revert with zero
        // data.
        _doCallReturnData(address(sc), payload, 0x200, 0x20);
        assembly {
            if iszero(eq(keccak256(0x60, 0x20), keccak256(0x200, returndatasize()))) { revert(0x00, 0x00) }
        }
    }

    /// @dev Tests that expanding memory outside of the range given to `expectSafeMemory`
    ///      will cause the test to fail while using the `REVERT` opcode.
    function testFailExpectSafeMemory_REVERT() public {
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

    ////////////////////////////////////////////////////////////////
    //                    Context Depth Tests                     //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that the `expectSafeMemory` cheatcode respects context depth while
    ///      using the `MSTORE` opcode.
    function testExpectSafeMemory_MSTORE_respectsDepth() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();
        // Create a payload to call `doMstore8` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.doMstore8.selector, 0x120, 0xc0ffee);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Should not revert- the `expectSafeMemory` cheatcode operates at a
        // per-depth level.
        _doCall(address(sc), payload);
    }

    /// @dev Tests that the `expectSafeMemory` cheatcode respects context depth while
    ///      using the `MSTORE8` opcode.
    function testExpectSafeMemory_MSTORE8_respectsDepth() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();
        // Create a payload to call `doMstore8` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.doMstore8.selector, 0x120, 0xFF);

        // Allow memory writes in the range of [0x80, 0x100) within this context
        vm.expectSafeMemory(0x80, 0x100);

        // Should not revert- the `expectSafeMemory` cheatcode operates at a
        // per-depth level.
        _doCall(address(sc), payload);
    }

    ////////////////////////////////////////////////////////////////
    //              `expectSafeMemoryCall` cheatcode              //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that the `expectSafeMemoryCall` cheatcode works as expected.
    function testExpectSafeMemoryCall() public {
        // Create a new SubContext contract
        SubContext sc = new SubContext();
        // Create a payload to call `doMstore8` on the SubContext contract
        bytes memory payload = abi.encodeWithSelector(SubContext.doMstore.selector, 0x80, 0xc0ffee);

        // Allow memory writes in the range of [0x80, 0xA0) within the next created subcontext
        vm.expectSafeMemoryCall(0x80, 0xA0);

        // Should not revert- the memory write in this subcontext is within the allowed range.
        _doCall(address(sc), payload);
    }

    /// @dev Tests that the `expectSafeMemoryCall` cheatcode works as expected.
    function testFailExpectSafeMemoryCall() public {
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

    ////////////////////////////////////////////////////////////////
    //              `stopExpectSafeMemory` cheatcode              //
    ////////////////////////////////////////////////////////////////

    /// @dev Tests that the `stopExpectSafeMemory` cheatcode works as expected.
    function testStopExpectSafeMemory() public {
        uint64 initPtr;
        assembly {
            initPtr := mload(0x40)
        }

        vm.expectSafeMemory(initPtr, initPtr + 0x20);
        assembly {
            // write to allowed range
            mstore(initPtr, 0x01)
        }

        vm.stopExpectSafeMemory();

        assembly {
            // write ouside allowed range, this should be fine
            mstore(add(initPtr, 0x20), 0x01)
        }
    }

    /// @dev Tests that the `stopExpectSafeMemory` cheatcode does not cause violations not being noticed.
    function testFailStopExpectSafeMemory() public {
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

    /// @dev Tests that the `stopExpectSafeMemory` cheatcode can still be called if the free memory pointer was
    ///      updated to the exclusive upper boundary during execution.
    function testStopExpectSafeMemory_freeMemUpdate() public {
        uint64 initPtr;
        assembly {
            initPtr := mload(0x40)
        }

        vm.expectSafeMemory(initPtr, initPtr + 0x20);
        assembly {
            // write outside of allowed range, this should revert
            mstore(initPtr, 0x01)
            mstore(0x40, add(initPtr, 0x20))
        }

        vm.stopExpectSafeMemory();
    }

    ////////////////////////////////////////////////////////////////
    //                          HELPERS                           //
    ////////////////////////////////////////////////////////////////

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
