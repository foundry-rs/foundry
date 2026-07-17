//@compile-flags: --only-lint reentrancy-events

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.15;

interface IExternal {
    function notify(uint256 value) external returns (bool);
    function peek() external view returns (uint256);
    function compute(uint256 x) external pure returns (uint256);
}

// Overloaded interface methods exercise the multi-match member lookup.
interface IBus {
    function notify(uint256) external returns (bool);
    function notify(uint256, bytes calldata) external returns (bool);
    function peek(uint256) external view returns (uint256);
    function peek(uint256, uint256) external view returns (uint256);
}

interface ICall {
    function poke() external;
}

contract Other {
    function action(uint256) external returns (bool) {
        return true;
    }
}

contract SuperTransferBase {
    function transfer(IExternal d) public virtual {
        d.notify(0);
    }
}

contract SuperTransferChild is SuperTransferBase {
    event Tick();

    function emitAfterSuperTransfer(IExternal d) external {
        super.transfer(d);
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }
}

contract RecursiveCacheKeyRepro {
    event Tick();

    ICall ext;
    bool once;

    function entry(bool flag) external {
        once = false;
        if (flag) {
            helperA();
            return;
        }
        noOp();
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function noOp() internal {
        if (!once) {
            once = true;
            helperA();
        }
    }

    function helperA() internal {
        noOp();
        ext.poke();
    }
}

contract ModifierArgReachabilityRepro {
    event Tick();

    bool flag;

    function ext() external returns (bool) {
        flag = true;
        return true;
    }

    modifier m(bool ok) {
        _;
    }

    function entry(uint256 depth, bool useCache) external {
        if (useCache) {
            f(depth);
            return;
        }
        c(depth);
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function f(uint256 depth) internal {
        if (depth > 0) {
            c(depth - 1);
        }
        hh();
    }

    function hh() internal m(this.ext()) {}

    function c(uint256 depth) internal {
        h(depth);
    }

    function h(uint256 depth) internal {
        f(depth);
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

    modifier emitSuffix() {
        _;
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

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

    function emitAfterYulCall(address target) external {
        assembly {
            pop(call(gas(), target, 0, 0, 0, 0, 0))
        }
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterYulStaticcall(address target) external {
        assembly {
            pop(staticcall(gas(), target, 0, 0, 0, 0))
        }
        emit Tick();
    }

    function emitAfterYulSwitchCall(address target, uint256 selector) external {
        assembly {
            switch selector
            case 0 { pop(call(gas(), target, 0, 0, 0, 0, 0)) }
            default {}
        }
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
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

    function emitAfterHelperHeavyFanout() external {
        _doExternalWork();
        _doExternalWork();
        _doExternalWork();
        _doExternalWork();
        _doExternalWork();
        _doExternalWork();
        _doExternalWork();
        _doExternalWork();
        _doExternalWork();
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

    function siblingOperandOrder(IExternal d) external {
        _consumePair(_emitAndReturn(), d.notify(1) ? 1 : 0);
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

    // Member-form internal calls (`Lib.f(...)`, `using for`) are resolved by Solar and followed.
    function emitAfterLibraryStaticCall() external {
        NotifyLib.staticHelper(ext);
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterUsingForCall() external {
        ext.notifyVia(1);
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
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

    // Ternary with one aborting branch: per-branch abort tracking must let the live
    // branch's taint survive so the subsequent emit is still flagged.
    function emitAfterTernaryAbortingElse(IExternal d, bool flag) external {
        uint256 x = flag ? _peeker(d) : _alwaysRevertsU();
        emit Counter(x); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterTernaryAbortingThen(IExternal d, bool flag) external {
        uint256 x = flag ? _alwaysRevertsU() : _peeker(d);
        emit Counter(x); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // Both ternary branches abort: the post-ternary emit is genuinely unreachable.
    function emitAfterTernaryBothAbort(bool flag) external {
        uint256 x = flag ? _alwaysRevertsU() : _alwaysRevertsU2();
        emit Counter(x);
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

    // Overloaded mutating external method: must flag despite the name collision in
    // the interface (member lookup used to drop overloads via a unique-name filter).
    function emitAfterOverloadedMutatingCall(IBus b) external {
        b.notify(0);
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function emitAfterOverloadedMutatingCallTwoArgs(IBus b) external {
        b.notify(0, "");
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // Overloaded but all overloads are `view` — must NOT flag.
    function emitAfterOverloadedViewCall(IBus b) external {
        uint256 v = b.peek(0);
        emit Counter(v);
    }

    function emitAfterOverloadedViewCallTwoArgs(IBus b) external {
        uint256 v = b.peek(0, 1);
        emit Counter(v);
    }

    // `this.<view|pure>()` compiles to STATICCALL — cannot reorder events.
    function emitAfterSelfViewCall() external {
        uint256 v = this.viewSelf();
        emit Counter(v);
    }

    function emitAfterSelfPureCall() external {
        uint256 v = this.pureSelf(1);
        emit Counter(v);
    }

    // Only the selected overload matters: the zero-arg self-call is pure, even though a
    // one-arg overload below is state-mutating.
    function emitAfterSelfPureOverloadCall() external {
        uint256 v = this.overloadedSelf();
        emit Counter(v);
    }

    // Mutating self-external call still taints subsequent emits.
    function emitAfterSelfMutatingCall() external {
        counter += 1;
        this.publicHelper();
        emit Counter(counter); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    // Caller is already tainted, then calls a helper that always reverts. The post-call
    // emit is unreachable and must NOT be flagged.
    function emitAfterTaintedAlwaysRevertsHelper(IExternal d) external {
        d.notify(0);
        _alwaysReverts();
        emit Tick();
    }

    // The shared helper `_helperEmitAfterCall` is invoked from two callers. Its tainted
    // emit must be reported exactly once (in the helper's self-pass), not duplicated per
    // caller.
    function callsSharedHelperA() external {
        _helperEmitAfterCall();
    }

    function callsSharedHelperB() external {
        _helperEmitAfterCall();
    }

    // --- Helpers ------------------------------------------------------------

    function publicHelper() public {
        counter += 1;
    }

    function viewSelf() public view returns (uint256) {
        return counter;
    }

    function pureSelf(uint256 a) public pure returns (uint256) {
        return a + 1;
    }

    function overloadedSelf() public pure returns (uint256) {
        return 1;
    }

    function overloadedSelf(uint256 a) public returns (uint256) {
        counter += a;
        return counter;
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

    function _alwaysRevertsU() internal pure returns (uint256) {
        revert("nope");
    }

    function _alwaysRevertsU2() internal pure returns (uint256) {
        revert("nope2");
    }

    function _peeker(IExternal d) internal returns (uint256) {
        d.notify(0);
        return 0;
    }

    function _internalHelper() internal {
        counter += 1;
    }

    function _emitAndReturn() internal returns (uint256) {
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
        return 1;
    }

    function _consumePair(uint256, uint256) internal pure {}

    function _pureHelper(uint256 a) internal pure returns (uint256) {
        return a + 1;
    }

    // Tainted emit inside a shared helper. The lint reports this once (in the helper's
    // own self-pass), not once per caller.
    function _helperEmitAfterCall() internal {
        ext.notify(0);
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function returnThroughModifier() external emitSuffix {
        ext.notify(0);
        return;
    }

    function recursiveBaseCase(IExternal d, uint256 depth) public {
        if (depth == 0) {
            d.notify(0);
            return;
        }
        recursiveBaseCase(d, depth - 1);
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }

    function caughtFailure(IExternal d) external {
        try d.notify(0) returns (bool) {} catch {
            emit Tick();
        }
    }

    function conditionlessContinue(IExternal d) external {
        for (;; d.notify(0)) {
            emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
            continue;
        }
    }
}

// `super.<member>(...)` is internal base-chain dispatch — must not panic the linter and
// must not be treated as an external call by itself, but external calls inside the
// resolved base function must still taint the caller.
contract Base {
    IExternal internal ext_;
    event Tick();

    function doExt() internal {
        ext_.notify(0);
    }

    function viewBase() internal view returns (uint256) {
        return 0;
    }

    function pureBase() internal pure returns (uint256) {
        return 1;
    }
}

contract Child is Base {
    event Counter(uint256 value);

    // Clean: super resolves to a base function with no external call.
    function emitAfterSuperViewCall() external {
        uint256 v = super.viewBase();
        emit Counter(v);
    }

    function emitAfterSuperPureCall() external {
        uint256 v = super.pureBase();
        emit Counter(v);
    }

    // Transitive: the base function makes an external call, so the post-super emit is
    // tainted exactly like a direct `_helper()` call would be.
    function emitAfterSuperTaintingCall() external {
        super.doExt();
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }
}

contract SuperBaseWithTaint {
    IExternal internal ext__;
    event Tick();

    function overridden() internal virtual {
        ext__.notify(0);
    }

    function arity(uint256) internal {
        ext__.notify(0);
    }
}

contract SuperPureOverride is SuperBaseWithTaint {
    function overridden() internal pure override {}

    function arity() internal pure {}
}

contract SuperChild is SuperPureOverride {
    event Counter(uint256 value);

    // Clean: `super.overridden()` dispatches to the immediate pure override, not the
    // older base implementation with the same signature that performs an external call.
    function emitAfterSuperPureOverrideCall() external {
        super.overridden();
        emit Tick();
    }

    // Clean: `super.arity()` dispatches to the zero-arg overload only; the mutating
    // one-arg overload in the older base must not taint this call.
    function emitAfterSuperPureArityCall() external {
        super.arity();
        emit Counter(0);
    }
}

contract DispatchBase {
    event DispatchTick();

    function dispatchHook(IExternal target) internal virtual {}

    function inheritedEntry(IExternal target) external {
        dispatchHook(target);
        emit DispatchTick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }
}

contract DispatchLeaf is DispatchBase {
    function dispatchHook(IExternal target) internal override {
        target.notify(0);
    }
}

contract InternalFunctionPointerEvents {
    IExternal internal target;
    event Tick();

    function noop() internal {}

    function notify() internal {
        target.notify(0);
    }

    function notifyAgain() internal {
        target.notify(1);
    }

    function harmlessPointer() external {
        function() internal callback = noop;
        callback();
        emit Tick();
    }

    function taintingPointer(bool alternate) external {
        function() internal callback = alternate ? notify : notifyAgain;
        callback();
        emit Tick(); //~WARN: event emitted after an external call; reentrancy can reorder or fabricate logs that off-chain consumers rely on
    }
}
