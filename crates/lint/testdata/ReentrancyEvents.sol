//@compile-flags: --only-lint reentrancy-events

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.15;

interface IExternal {
    function notify(uint256 value) external returns (bool);
    function peek() external view returns (uint256);
    function compute(uint256 x) external pure returns (uint256);
}

contract Other {
    function action(uint256) external returns (bool) {
        return true;
    }
}

library NotifyLib {
    function notifyVia(IExternal d, uint256 v) internal {
        d.notify(v);
    }

    function staticHelper(IExternal d) internal {
        d.notify(0);
    }
}

contract ReentrancyEvents {
    using NotifyLib for IExternal;

    event Counter(uint256 value);
    event Paid(address indexed to, uint256 amount);
    event Tick();

    uint256 public counter;
    address payable public recipient;
    IExternal public ext;
    Other public other;

    // --- Bad cases ----------------------------------------------------------

    function emitAfterHighLevelCall(IExternal d) external {
        counter += 1;
        d.notify(counter);
        emit Counter(counter); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterLowLevelCall(address target) external {
        counter += 1;
        // forge-lint: disable-next-line(unchecked-call)
        target.call("");
        emit Counter(counter); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterTransfer() external {
        counter += 1;
        recipient.transfer(1 wei);
        emit Counter(counter); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterSend() external {
        counter += 1;
        // forge-lint: disable-next-line(unchecked-call)
        recipient.send(1 wei);
        emit Counter(counter); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterSelfExternalCall() external {
        counter += 1;
        this.publicHelper();
        emit Counter(counter); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitInIfBranch(IExternal d, bool flag) external {
        d.notify(0);
        if (flag) {
            emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
        }
    }

    function emitAfterCallInBranch(IExternal d, bool flag) external {
        if (flag) {
            d.notify(1);
        }
        // The "if" branch may have made an external call before reaching this emit.
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitInLoopAfterCall(IExternal d, uint256 n) external {
        d.notify(0);
        for (uint256 i; i < n; ++i) {
            emit Counter(i); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
        }
    }

    function emitAfterLoopWithCall(IExternal d, uint256 n) external {
        for (uint256 i; i < n; ++i) {
            d.notify(i);
        }
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // Loop back-edge: the emit is clean on iteration 1 but tainted by the call from the
    // previous iteration on iterations 2..N. The two-pass fixpoint must catch this.
    function emitInLoopBeforeCall(IExternal d, uint256 n) external {
        for (uint256 i; i < n; ++i) {
            emit Counter(i); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
            d.notify(i);
        }
    }

    function emitAfterTryCall(IExternal d) external {
        try d.notify(1) returns (bool) {} catch {}
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterHelperWithCall() external {
        _doExternalWork();
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterChainedCall() external {
        counter += 1;
        Other(other).action(counter);
        emit Counter(counter); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // External call inside the emit's arguments (Solidity evaluates args before emitting).
    function emitWithExternalCallInArgs(IExternal d) external {
        emit Counter(d.notify(1) ? 1 : 0); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // Internal helper returns early after an external call: caller should still see the taint.
    function emitAfterHelperEarlyReturn(bool flag) external {
        _maybeNotify(flag);
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // External call in one loop iteration must taint the post-loop state via `break`.
    function emitAfterBreakWithCall(IExternal d, bool flag) external {
        for (uint256 i; i < 10; ++i) {
            if (flag) {
                d.notify(i);
                break;
            }
        }
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // `new Foo(...)` deploys and runs a constructor — an external interaction.
    function emitAfterNew() external {
        new Other();
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // Same modifier listed twice — the second instance must not be silently dropped.
    modifier maybeNotify(bool b) {
        if (b) ext.notify(1);
        _;
    }

    function emitAfterDuplicateModifier() external maybeNotify(false) maybeNotify(true) {
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // Known limitation: member-form internal calls (`Lib.f(...)`, `using for`, `super.f()`)
    // are not yet followed because Solar's `members_of` for `TyKind::Type(Contract)` is a
    // TODO. The external call inside `staticHelper` is therefore missed and no warning fires.
    function emitAfterLibraryStaticCall() external {
        NotifyLib.staticHelper(ext);
        emit Tick();
    }

    function emitAfterUsingForCall() external {
        ext.notifyVia(1);
        emit Tick();
    }

    // --- Good cases ---------------------------------------------------------

    function emitBeforeExternalCall(IExternal d) external {
        counter += 1;
        emit Counter(counter);
        d.notify(counter);
    }

    function emitOnlyNoCalls() external {
        counter += 1;
        emit Counter(counter);
    }

    function emitAfterInternalOnly() external {
        _internalHelper();
        emit Tick();
    }

    function emitAfterPureInternalCall() external {
        uint256 x = _pureHelper(1);
        emit Counter(x);
    }

    function bothBranchesEmitBeforeCall(IExternal d, bool flag) external {
        if (flag) {
            emit Tick();
        } else {
            emit Counter(0);
        }
        d.notify(1);
    }

    // Internal helper that always reverts: caller's post-call code is unreachable, no warning.
    function emitAfterAlwaysAbortingHelper() external {
        _alwaysReverts();
        emit Tick();
    }

    // External call inside a loop body followed by a `revert` in the same iteration:
    // post-loop state must not be tainted because every body path aborts.
    function emitAfterLoopThatAlwaysReverts(IExternal d) external {
        for (uint256 i; i < 1; ++i) {
            d.notify(i);
            revert("oops");
        }
        emit Tick();
    }

    // External call only on a branch that itself returns early: the function's caller
    // sees the call, but within this function the post-if fallthrough state is clean,
    // so an emit on the fallthrough path is fine.
    function emitOnUntaintedBranch(IExternal d, bool flag) external {
        if (flag) {
            d.notify(0);
            return;
        }
        emit Tick();
    }

    // `selfdestruct(...)` terminates execution; post-statements are unreachable.
    function emitAfterSelfdestruct(IExternal d) external {
        d.notify(0);
        selfdestruct(payable(msg.sender));
        emit Tick();
    }

    // `require(false, ...)` and `assert(false)` also abort unconditionally.
    function emitAfterRequireFalse(IExternal d) external {
        d.notify(0);
        require(false, "no");
        emit Tick();
    }

    function emitAfterAssertFalse(IExternal d) external {
        d.notify(0);
        assert(false);
        emit Tick();
    }

    // `staticcall` cannot emit logs or perform state-changing reentrancy.
    function emitAfterStaticcall(address target) external {
        // forge-lint: disable-next-line(unchecked-call)
        target.staticcall("");
        emit Tick();
    }

    // High-level `view` external calls are read-only and cannot reorder events.
    function emitAfterViewCall(IExternal d) external {
        uint256 v = d.peek();
        emit Counter(v);
    }

    // High-level `pure` external calls likewise cannot reorder events.
    function emitAfterPureCall(IExternal d) external {
        uint256 v = d.compute(1);
        emit Counter(v);
    }

    // --- Helpers ------------------------------------------------------------

    function publicHelper() public {
        counter += 1;
    }

    function _doExternalWork() internal {
        ext.notify(counter);
    }

    function _maybeNotify(bool flag) internal {
        if (flag) {
            ext.notify(1);
            return;
        }
    }

    function _alwaysReverts() internal {
        ext.notify(0);
        revert("nope");
    }

    function _internalHelper() internal {
        counter += 1;
    }

    function _pureHelper(uint256 a) internal pure returns (uint256) {
        return a + 1;
    }
}
