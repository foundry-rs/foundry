// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "utils/Test.sol";

contract MStoreAndMLoadCaller {
    uint256 public constant expectedValueInMemory = 999;

    uint256 public memPtr; // the memory pointer being used

    function storeAndLoadValueFromMemory() public returns (uint256) {
        uint256 mPtr;
        assembly {
            mPtr := mload(0x40) // load free pointer
            mstore(mPtr, expectedValueInMemory)
            mstore(0x40, add(mPtr, 0x20))
        }

        // record & expose the memory pointer location
        memPtr = mPtr;

        uint256 result = 123;
        assembly {
            // override with `expectedValueInMemory`
            result := mload(mPtr)
        }
        return result;
    }
}

contract FirstLayer {
    SecondLayer secondLayer;

    constructor(SecondLayer _secondLayer) {
        secondLayer = _secondLayer;
    }

    function callSecondLayer() public view returns (uint256) {
        return secondLayer.endHere();
    }
}

contract SecondLayer {
    uint256 public constant endNumber = 123;

    function endHere() public view returns (uint256) {
        return endNumber;
    }
}

contract OutOfGas {
    uint256 dummyVal = 0;

    function consumeGas() public {
        dummyVal += 1;
    }

    function triggerOOG() public {
        bytes memory encodedFunctionCall = abi.encodeWithSignature("consumeGas()", "");
        uint256 notEnoughGas = 50;
        (bool success,) = address(this).call{gas: notEnoughGas}(encodedFunctionCall);
        require(!success, "it should error out of gas");
    }
}

contract RecordDebugTraceTest is Test {
    /**
     * The goal of this test is to ensure the debug steps provide the correct OPCODE with its stack
     * and memory input used. The test checke MSTORE and MLOAD and ensure it records the expected
     * stack and memory inputs.
     */
    function testDebugTraceCanRecordOpcodeWithStackAndMemoryData() public {
        MStoreAndMLoadCaller testContract = new MStoreAndMLoadCaller();

        vm.startDebugTraceRecording();

        uint256 val = testContract.storeAndLoadValueFromMemory();
        assertTrue(val == testContract.expectedValueInMemory());

        Vm.DebugStep[] memory steps = vm.stopAndReturnDebugTraceRecording();

        bool mstoreCalled = false;
        bool mloadCalled = false;

        for (uint256 i = 0; i < steps.length; i++) {
            Vm.DebugStep memory step = steps[i];
            if (
                step.opcode == 0x52 /*MSTORE*/
                    && step.stack[0] == testContract.memPtr() // MSTORE offset
                    && step.stack[1] == testContract.expectedValueInMemory() // MSTORE val
            ) {
                mstoreCalled = true;
            }

            if (
                step.opcode == 0x51 /*MLOAD*/
                    && step.stack[0] == testContract.memPtr() // MLOAD offset
                    && step.memoryInput.length == 32 // MLOAD should always load 32 bytes
                    && uint256(bytes32(step.memoryInput)) == testContract.expectedValueInMemory() // MLOAD value
            ) {
                mloadCalled = true;
            }
        }

        assertTrue(mstoreCalled);
        assertTrue(mloadCalled);
    }

    /**
     * This test tests that the cheatcode can correctly record the depth of the debug steps.
     * This is test by test -> FirstLayer -> SecondLayer and check that the
     * depth of the FirstLayer and SecondLayer are all as expected.
     */
    function testDebugTraceCanRecordDepth() public {
        SecondLayer second = new SecondLayer();
        FirstLayer first = new FirstLayer(second);

        vm.startDebugTraceRecording();

        first.callSecondLayer();

        Vm.DebugStep[] memory steps = vm.stopAndReturnDebugTraceRecording();

        bool goToDepthTwo = false;
        bool goToDepthThree = false;
        for (uint256 i = 0; i < steps.length; i++) {
            Vm.DebugStep memory step = steps[i];

            if (step.depth == 2) {
                assertTrue(step.contractAddr == address(first), "must be first layer on depth 2");
                goToDepthTwo = true;
            }

            if (step.depth == 3) {
                assertTrue(step.contractAddr == address(second), "must be second layer on depth 3");
                goToDepthThree = true;
            }
        }
        assertTrue(goToDepthTwo && goToDepthThree, "must have been to both first and second layer");
    }

    /**
     * The goal of this test is to ensure it can return expected `isOutOfGas` flag.
     * It is tested with out of gas result here.
     */
    function testDebugTraceCanRecordOutOfGas() public {
        OutOfGas testContract = new OutOfGas();

        vm.startDebugTraceRecording();

        testContract.triggerOOG();

        Vm.DebugStep[] memory steps = vm.stopAndReturnDebugTraceRecording();

        bool isOOG = false;
        for (uint256 i = 0; i < steps.length; i++) {
            Vm.DebugStep memory step = steps[i];

            if (step.isOutOfGas) {
                isOOG = true;
            }
        }
        assertTrue(isOOG, "should OOG");
    }
}
