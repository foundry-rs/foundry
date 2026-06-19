//@compile-flags: --only-lint require-revert-in-loop

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

error BadItem(uint256 index);

contract RequireRevertInLoop {
    function requireInsideLoop(uint256[] calldata values) external pure {
        for (uint256 i; i < values.length; ++i) {
            require(values[i] != 0, "zero"); //~WARN: `require` or `revert` inside a loop
        }
    }

    function revertInsideLoop(uint256[] calldata values) external pure {
        uint256 i;
        while (i < values.length) {
            if (values[i] == 0) {
                revert BadItem(i); //~WARN: `require` or `revert` inside a loop
            }
            ++i;
        }
    }

    function builtinRevertInsideLoop(uint256[] calldata values) external pure {
        for (uint256 i; i < values.length; ++i) {
            if (values[i] == 0) {
                revert("zero"); //~WARN: `require` or `revert` inside a loop
            }
        }
    }

    function helperRequireInsideLoop(uint256[] calldata values) external pure {
        for (uint256 i; i < values.length; ++i) {
            validate(values[i]);
        }
    }

    function validate(uint256 value) internal pure {
        require(value != 0, "zero"); //~WARN: `require` or `revert` inside a loop
    }

    function helperRevertInsideLoop(uint256[] calldata values) external pure {
        for (uint256 i; i < values.length; ++i) {
            validateWithRevert(values[i], i);
        }
    }

    function validateWithRevert(uint256 value, uint256 index) internal pure {
        if (value == 0) {
            revert BadItem(index); //~WARN: `require` or `revert` inside a loop
        }
    }

    function helperWithOwnLoopCalledOutsideLoop(uint256[] calldata values) external pure {
        validateLoop(values);
    }

    function helperWithOwnLoopCalledInsideLoop(uint256[][] calldata batches) external pure {
        for (uint256 i; i < batches.length; ++i) {
            validateLoop(batches[i]);
        }
    }

    function validateLoop(uint256[] calldata values) internal pure {
        for (uint256 i; i < values.length; ++i) {
            require(values[i] != 0, "zero"); //~WARN: `require` or `revert` inside a loop
        }
    }

    modifier repeated(uint256 iterations) {
        for (uint256 i; i < iterations; ++i) {
            _;
        }
    }

    function modifierPlaceholderLoop(uint256 iterations, bool ok) external pure repeated(iterations) {
        require(ok, "not ok"); //~WARN: `require` or `revert` inside a loop
    }

    function yulRevertInsideLoop(uint256 iterations) external pure {
        for (uint256 i; i < iterations; ++i) {
            assembly {
                revert(0, 0) //~WARN: `require` or `revert` inside a loop
            }
        }
    }

    function requireOutsideLoop(uint256 value) external pure {
        require(value != 0, "zero");
    }

    function helperRequireOutsideLoop(uint256 value) external pure {
        validateOutsideLoop(value);
    }

    function validateOutsideLoop(uint256 value) internal pure {
        require(value != 0, "zero");
    }

    function loopWithoutRequireOrRevert(uint256[] calldata values) external pure returns (uint256 sum) {
        for (uint256 i; i < values.length; ++i) {
            if (values[i] == 0) {
                continue;
            }
            sum += values[i];
        }
    }
}
