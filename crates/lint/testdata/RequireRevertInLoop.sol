//@compile-flags: --only-lint require-revert-in-loop

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

error BadItem(uint256 index);

library RequireRevertInLoopLib {
    function validateExtension(uint256 value) internal pure {
        require(value != 0, "zero"); //~WARN: `require` or `revert` inside a loop
    }
}

contract RequireRevertInLoop {
    using RequireRevertInLoopLib for uint256;

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

    function sharedHelperRequireInsideFirstLoop(uint256[] calldata values) external pure {
        for (uint256 i; i < values.length; ++i) {
            sharedValidate(values[i]);
        }
    }

    function sharedHelperRequireInsideSecondLoop(uint256[] calldata values) external pure {
        for (uint256 i; i < values.length; ++i) {
            sharedValidate(values[i]);
        }
    }

    function sharedValidate(uint256 value) internal pure {
        require(value != 0, "zero");
        //~^WARN: `require` or `revert` inside a loop
    }

    function requireInLoopCondition(uint256 iterations) external pure {
        uint256 i;
        while (conditionWithRequire(i, iterations)) {
            ++i;
        }
    }

    function conditionWithRequire(uint256 i, uint256 iterations) internal pure returns (bool) {
        require(iterations < 100, "too many"); //~WARN: `require` or `revert` inside a loop
        return i < iterations;
    }

    function requireInForLoopUpdate(uint256 iterations) external pure {
        for (uint256 i; i < iterations; i = incrementWithRequire(i)) {}
    }

    function incrementWithRequire(uint256 i) internal pure returns (uint256) {
        require(i < 100, "too many"); //~WARN: `require` or `revert` inside a loop
        return i + 1;
    }

    function extensionRequireInsideLoop(uint256[] calldata values) external pure {
        for (uint256 i; i < values.length; ++i) {
            values[i].validateExtension();
        }
    }

    function externalSelfCallInsideLoop(uint256[] calldata values) external view {
        for (uint256 i; i < values.length; ++i) {
            this.externalValidate(values[i]);
        }
    }

    function externalValidate(uint256 value) external pure {
        require(value != 0, "zero");
    }

    function overloadedHelperInsideLoop(uint256[] calldata values) external pure {
        for (uint256 i; i < values.length; ++i) {
            overloadedValidate(values[i]);
        }
    }

    function overloadedValidate(uint256 value) internal pure {
        require(value != 0, "zero"); //~WARN: `require` or `revert` inside a loop
    }

    function overloadedValidate(address value) internal pure {
        require(value != address(0), "zero");
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
