//@compile-flags: --only-lint msg-value-loop

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract MsgValueBase {
    function superRead() internal virtual returns (uint256) {
        return 0;
    }
}

contract MsgValueReader is MsgValueBase {
    function superRead() internal virtual override returns (uint256) {
        return msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
    }
}

contract MsgValueSuperCaller is MsgValueBase {
    function callNext() internal returns (uint256) {
        return super.superRead();
    }
}

contract MsgValueSuperOverloadBase {
    function overloaded(uint256) internal virtual returns (uint256) {
        return msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
    }
}

contract MsgValueSuperOverloadMask is MsgValueSuperOverloadBase {
    function overloaded() internal pure returns (uint256) {
        return 0;
    }
}

contract MsgValueSuperOverloadLeaf is MsgValueSuperOverloadMask {
    uint256 public total;

    function payableLoopWithOverloadedSuperMsgValue(uint256 iterations) external payable {
        for (uint256 i; i < iterations; ++i) {
            total += super.overloaded(i);
        }
    }
}

library MsgValueExtension {
    function extensionRead(uint256 self) internal returns (uint256) {
        self;
        return msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
    }
}

library MsgValueUnusedExtension {
    function extensionRead(uint256 self) internal returns (uint256) {
        self;
        return msg.value;
    }
}

contract MsgValueExternalReader {
    function publicReadValue() public payable returns (uint256) {
        return msg.value;
    }
}

contract MsgValueLoop is MsgValueReader, MsgValueSuperCaller {
    using MsgValueExtension for uint256;

    uint256 public total;

    constructor(uint256 iterations) payable {
        for (uint256 i; i < iterations; ++i) {
            total += msg.value;
        }
    }

    receive() external payable {
        for (uint256 i; i < 2; ++i) {
            total += msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
        }
    }

    fallback() external payable {
        for (uint256 i; i < 2; ++i) {
            total += msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
        }
    }

    function superRead() internal override(MsgValueBase, MsgValueReader) returns (uint256) {
        return super.superRead();
    }

    function payableForLoop(uint256 iterations) external payable {
        for (uint256 i; i < iterations; ++i) {
            total += msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
        }
    }

    function payableWhileLoop(uint256 iterations) external payable {
        uint256 i;
        while (i < iterations) {
            total += msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
            ++i;
        }
    }

    function payableDoWhileLoop(uint256 iterations) external payable {
        if (iterations == 0) return;
        uint256 i;
        do {
            total += msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
            ++i;
        } while (i < iterations);
    }

    function payableForUpdateExpression(uint256 iterations) external payable {
        uint256 value;
        for (uint256 i; i < iterations; value = msg.value + i++) {} //~WARN: payable functions should not use `msg.value` inside a loop
        total += value;
    }

    modifier loopPlaceholder(uint256 iterations) {
        for (uint256 i; i < iterations; ++i) {
            _;
        }
    }

    function payableModifierLoopPlaceholder(uint256 iterations) external payable loopPlaceholder(iterations) {
        total += msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
    }

    function payableLoopWithInternalMsgValue(uint256 iterations) external payable {
        for (uint256 i; i < iterations; ++i) {
            total += readValue();
        }
    }

    function readValue() internal returns (uint256) {
        return msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
    }

    function payableInternalLoopWithMsgValue(uint256 iterations) external payable {
        readValueInLoop(iterations);
    }

    function readValueInLoop(uint256 iterations) internal {
        for (uint256 i; i < iterations; ++i) {
            total += msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
        }
    }

    function payableLoopWithPublicMsgValue(uint256 iterations) external payable {
        for (uint256 i; i < iterations; ++i) {
            total += publicReadValue();
        }
    }

    function publicReadValue() public payable returns (uint256) {
        return msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
    }

    function payableLoopWithSuperMsgValue(uint256 iterations) external payable {
        for (uint256 i; i < iterations; ++i) {
            total += callNext();
        }
    }

    function payableLoopWithExtensionMsgValue(uint256 iterations) external payable {
        uint256 value = iterations;
        for (uint256 i; i < iterations; ++i) {
            total += value.extensionRead();
        }
    }

    function payableLoopWithExternalReader(MsgValueExternalReader reader, uint256 iterations) external payable {
        for (uint256 i; i < iterations; ++i) {
            total += reader.publicReadValue();
        }
    }

    function payableLoopWithDuplicateHelperA(uint256 iterations) external payable {
        for (uint256 i; i < iterations; ++i) {
            total += duplicateReadValue();
        }
    }

    function payableLoopWithDuplicateHelperB(uint256 iterations) external payable {
        for (uint256 i; i < iterations; ++i) {
            total += duplicateReadValue();
        }
    }

    function duplicateReadValue() internal returns (uint256) {
        return msg.value; //~WARN: payable functions should not use `msg.value` inside a loop
    }

    function payableMsgValueOutsideLoop() external payable {
        total += msg.value;
    }

    function payableCachedValueInLoop(uint256 iterations) external payable {
        uint256 value = msg.value;
        for (uint256 i; i < iterations; ++i) {
            total += value;
        }
    }

    function nonPayableLoop(uint256 iterations) external {
        nonPayableValueLoop(iterations);
    }

    function nonPayableValueLoop(uint256 iterations) internal {
        for (uint256 i; i < iterations; ++i) {
            total += msg.value;
        }
    }
}
