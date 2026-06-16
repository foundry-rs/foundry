//@compile-flags: --only-lint incorrect-modifier

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

contract IncorrectModifier {
    bool enabled;
    bool paused;

    modifier conditionalPlaceholder() { //~WARN: modifier can finish without executing the modified function
        if (enabled) {
            _;
        }
    }

    modifier returnsBeforePlaceholder() { //~WARN: modifier can finish without executing the modified function
        if (paused) {
            return;
        }
        _;
    }

    modifier nestedConditionalPlaceholder() { //~WARN: modifier can finish without executing the modified function
        if (enabled) {
            if (!paused) {
                _;
            }
        } else {
            revert("disabled");
        }
    }

    modifier loopOnlyPlaceholder() { //~WARN: modifier can finish without executing the modified function
        while (enabled) {
            _;
        }
    }

    modifier alwaysExecutes() {
        _;
    }

    modifier revertsOrExecutes() {
        if (!enabled) {
            revert("disabled");
        }
        _;
    }

    modifier conditionalElseReverts() {
        if (enabled) {
            _;
        } else {
            revert("disabled");
        }
    }

    modifier requireFalseOrExecutes() {
        if (paused) {
            require(false, "disabled");
            return;
        }
        _;
    }

    modifier assertFalseOrExecutes() {
        if (paused) {
            assert(false);
            return;
        }
        _;
    }

    modifier afterPlaceholderIsIrrelevant() {
        _;
        if (paused) {
            return;
        }
    }

    // A `do/while` body always runs at least once, so `_` is always reached.
    modifier doWhileAlwaysRuns() {
        do {
            _;
        } while (enabled);
    }

    // A loop before an unconditional placeholder still always reaches `_`.
    modifier loopBeforePlaceholder() {
        while (enabled) {
            enabled = false;
        }
        _;
    }

    modifier forLoopBeforePlaceholder() {
        for (uint256 i = 0; i < 3; i++) {
            enabled = false;
        }
        _;
    }

    // Every try/catch path reaches `_` or reverts.
    modifier tryCatchCovered() {
        try this.poke() {
            _;
        } catch {
            revert("failed");
        }
    }

    // A `break` that skips `_` with no later placeholder is still flagged.
    modifier breakSkipsPlaceholder() { //~WARN: modifier can finish without executing the modified function
        while (enabled) {
            if (paused) {
                break;
            }
            _;
        }
    }

    // A `do/while` body runs once, but a `continue` reaches the trailing condition, which can then
    // exit the loop before `_`.
    modifier doWhileContinueSkipsPlaceholder() { //~WARN: modifier can finish without executing the modified function
        do {
            if (paused) {
                continue;
            }
            _;
        } while (enabled);
    }

    // Assembly that always reverts before `_` reaches no normal exit, so it is not flagged.
    modifier yulRevertOrExecutes() {
        if (!enabled) {
            assembly {
                revert(0, 0)
            }
        }
        _;
    }

    // A `return(...)` in assembly halts successfully without running `_`, so it is flagged.
    modifier yulReturnBeforePlaceholder() { //~WARN: modifier can finish without executing the modified function
        assembly {
            return(0, 0)
        }
        _;
    }

    function poke() external {}
}
