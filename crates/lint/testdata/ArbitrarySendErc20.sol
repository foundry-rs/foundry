//@compile-flags: --only-lint arbitrary-send-erc20

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function approve(address spender, uint256 amount) external returns (bool);
    function permit(
        address owner,
        address spender,
        uint256 value,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external;
}

interface IERC721 {
    // Same name as ERC20.transferFrom but a different ABI; should NOT match.
    function transferFrom(address from, address to, uint256 tokenId) external;
    function safeTransferFrom(address from, address to, uint256 tokenId) external;
    function safeTransferFrom(address from, address to, uint256 tokenId, bytes calldata data) external;
}

interface IERC3156FlashBorrower {
    function onFlashLoan(
        address initiator,
        address token,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) external returns (bytes32);
}

// Same method name as EIP-3156, different signature.
interface IFakeFlashBorrower {
    function onFlashLoan(bytes calldata data) external;
}

library SafeERC20 {
    function safeTransferFrom(
        IERC20 token,
        address from,
        address to,
        uint256 value
    ) internal {
        // Library passthrough — `from` is forwarded by the caller; the lint fires at the
        // user's call site (see `badLibrary`), not here.
        require(token.transferFrom(from, to, value), "SafeERC20: transferFrom failed");
    }

    function safePermit(
        IERC20 token,
        address owner,
        address spender,
        uint256 value,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) internal {
        token.permit(owner, spender, value, deadline, v, r, s);
    }
}

contract ArbitrarySendErc20 {
    using SafeERC20 for IERC20;

    IERC20 public token;
    IERC20 public other;
    IERC721 public nft;
    address public owner;
    address public immutable trustedOwner;

    constructor(address _trustedOwner) {
        trustedOwner = _trustedOwner;
    }

    function _msgSender() internal view returns (address) {
        return msg.sender;
    }

    // Verifies depth-bounded helper recognition.
    function _origin() internal view returns (address) {
        return _msgSender();
    }

    // -- POSITIVE CASES (should warn) --

    function badPlain(address from, address to, uint256 a) public {
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badSafeMember(address from, address to, uint256 a) public {
        token.safeTransferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badLibrary(address from, address to, uint256 a) public {
        SafeERC20.safeTransferFrom(token, from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Reassignment kills earlier safety.
    function badReassign(address from, address to, uint256 a) public {
        address x = msg.sender;
        x = from;
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Ternary with one unsafe branch.
    function badTernary(bool flag, address from, address to, uint256 a) public {
        address x = flag ? msg.sender : from;
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Tuple destructuring picks the unsafe slot.
    function badTuple(address from, address to, uint256 a) public {
        (, address x) = (msg.sender, from);
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Index reads are opaque.
    function badIndex(address[] calldata senders, address to, uint256 a) public {
        token.transferFrom(senders[0], to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badPermitWrongToken(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        other.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badPermitWrongSpender(
        address from,
        address spender,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, spender, a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badPermitAfter(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
        token.permit(from, address(this), a, deadline, v, r, s);
    }

    // Permit on a side branch must not suppress the fall-through path.
    function badPermitInOtherBranch(
        bool flag,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        if (flag) {
            token.permit(from, address(this), a, deadline, v, r, s);
        }
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Token reassigned after permit invalidates the record.
    function badPermitTokenReassigned(
        IERC20 t,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        t.permit(from, address(this), a, deadline, v, r, s);
        t = other;
        t.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Owner reassigned after permit invalidates the record.
    function badPermitOwnerReassigned(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address x = from;
        token.permit(x, address(this), a, deadline, v, r, s);
        x = to;
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Disjunction does not establish equality.
    function badDisjunction(address from, address to, uint256 a) public {
        require(from == msg.sender || to == msg.sender, "weak");
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Guard scoped to one branch must not leak.
    function badGuardScoped(bool flag, address from, address to, uint256 a) public {
        if (flag) {
            require(from == msg.sender, "ok in this branch");
        }
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badInterfaceCast(address rawToken, address from, address to, uint256 a) public {
        IERC20(rawToken).transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Sink runs before the equality short-circuits — the guard cannot retroactively
    // sanitize `from`.
    function badRequireGuardOrder(address from, address to, uint256 a) public {
        require(token.transferFrom(from, to, a) && from == msg.sender); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badAssertGuardOrder(address from, address to, uint256 a) public {
        assert(token.transferFrom(from, to, a) && from == msg.sender); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badShortCircuitReassignKillsSafe(bool flag, address from, address to, uint256 a) public {
        address x = msg.sender;
        bool ok = flag && ((x = from) != address(0));
        ok;
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Modifier guard placed *after* `_;` cannot be hoisted.
    modifier lateCheck(address f) {
        _;
        require(f == msg.sender, "auth");
    }

    function badModifierGuardAfterPlaceholder(
        address from,
        address to,
        uint256 a
    ) public lateCheck(from) {
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Multi-`_;` modifier is skipped (placeholder ordering can't be assumed sound).
    modifier multiPlaceholder(address f) {
        _;
        require(f == msg.sender, "auth");
        _;
    }

    function badModifierMultiPlaceholder(
        address from,
        address to,
        uint256 a
    ) public multiPlaceholder(from) {
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Loop body kills a previously-safe local.
    function badLoopKillsSafeLocal(address from, address to, uint256 a) public {
        address x = msg.sender;
        for (uint256 i = 0; i < 1; i++) {
            x = from;
        }
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Try clause kills a previously-safe local.
    function badTryClauseKillsSafeLocal(
        address from,
        address to,
        uint256 a,
        IERC20 t
    ) public {
        address x = msg.sender;
        try t.transfer(to, a) returns (bool) {
            x = from;
        } catch {}
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Mutable state vars are not transitive: storage may be rewritten before the sink.
    function badViaStateVarGuard(address from, address to, uint256 a) public {
        require(owner == msg.sender, "owner check");
        require(from == owner, "from check");
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // State-var token reassigned after permit must invalidate the record.
    function badPermitStateTokenReassigned(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        token = other;
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // State-var owner reassigned after permit must invalidate the record.
    function badPermitStateOwnerReassigned(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        owner = from;
        token.permit(owner, address(this), a, deadline, v, r, s);
        owner = to;
        token.transferFrom(owner, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // State-var token reassigned on only one branch: post-`if` intersection drops the permit.
    function badPermitStateTokenMaybeReassigned(
        bool flag,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        if (flag) {
            token = other;
        }
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // -- NEGATIVE CASES (should NOT warn) --

    function okMsgSender(address to, uint256 a) public {
        token.transferFrom(msg.sender, to, a);
    }

    function okThis(address to, uint256 a) public {
        token.transferFrom(address(this), to, a);
    }

    function okTransitive(address to, uint256 a) public {
        address tmp = address(msg.sender);
        token.transferFrom(tmp, to, a);
    }

    function okPayableCast(address to, uint256 a) public {
        token.transferFrom(payable(msg.sender), to, a);
    }

    function okRequireEq(address from, address to, uint256 a) public {
        require(from == msg.sender, "auth");
        token.transferFrom(from, to, a);
    }

    function okAssertEq(address from, address to, uint256 a) public {
        assert(from == msg.sender);
        token.transferFrom(from, to, a);
    }

    function okConjunction(address from, address to, uint256 a) public {
        require(from == msg.sender && to != address(0), "auth");
        token.transferFrom(from, to, a);
    }

    // Short-circuit: the equality holds by the time the sink runs.
    function okRequireShortCircuit(address from, address to, uint256 a) public {
        require(from == msg.sender && token.transferFrom(from, to, a));
    }

    function okIfRevert(address from, address to, uint256 a) public {
        if (from != msg.sender) revert("auth");
        token.transferFrom(from, to, a);
    }

    function okParens(address to, uint256 a) public {
        token.transferFrom((msg.sender), to, a);
    }

    function okTernaryBothSafe(bool flag, address to, uint256 a) public {
        address x = flag ? msg.sender : address(this);
        token.transferFrom(x, to, a);
    }

    function okTuple(address to, uint256 a, address ignored) public {
        (address x, ) = (msg.sender, ignored);
        token.transferFrom(x, to, a);
    }

    modifier onlySelf(address f) {
        require(f == msg.sender, "auth");
        _;
    }

    function okModifier(address from, address to, uint256 a) public onlySelf(from) {
        token.transferFrom(from, to, a);
    }

    // Mutable storage modifier-arg reassigned before the sink.
    function badModifierMutableState(address from, address to, uint256 a) public onlySelf(owner) {
        owner = from;
        token.transferFrom(owner, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // OpenZeppelin's `_msgSender()` resolves to `msg.sender`.
    function okMsgSenderHelper(address to, uint256 a) public {
        token.transferFrom(_msgSender(), to, a);
    }

    // Helper chain `_origin -> _msgSender -> msg.sender` within depth budget.
    function okMsgSenderHelperChain(address to, uint256 a) public {
        token.transferFrom(_origin(), to, a);
    }

    function okMsgSenderHelperGuard(address from, address to, uint256 a) public {
        require(from == _msgSender(), "auth");
        token.transferFrom(from, to, a);
    }

    // `assert(false)` is recognised as an exit.
    function okIfAssertFalse(address from, address to, uint256 a) public {
        if (from != msg.sender) {
            assert(false);
        }
        token.transferFrom(from, to, a);
    }

    // `immutable` state vars can chain: storage cannot be rewritten post-deploy.
    function okImmutableOwnerChain(address from, address to, uint256 a) public {
        require(trustedOwner == msg.sender, "owner");
        require(from == trustedOwner, "from");
        token.transferFrom(from, to, a);
    }

    // Same token, spender == this, owner matches `from`, no reassignment.
    function okPermit(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // ERC721 same-named methods must NOT trigger this lint.
    function okErc721TransferFrom(address from, address to, uint256 id) public {
        nft.transferFrom(from, to, id);
        nft.safeTransferFrom(from, to, id);
        nft.safeTransferFrom(from, to, id, "");
    }

    // View / pure functions are out of scope.
    function viewMethodsExempt(address from, address to, uint256 a) external view returns (uint256) {
        from;
        to;
        a;
        return 0;
    }

    // Reassignment that *establishes* safety.
    function okReassignToSafe(address from, address to, uint256 a) public {
        address x = from;
        x = msg.sender;
        token.transferFrom(x, to, a);
    }

    function okLibrarySelf(address to, uint256 a) public {
        SafeERC20.safeTransferFrom(token, address(this), to, a);
    }

    // Explicit `else` inherits the negated guard.
    function okExplicitElseGuard(address from, address to, uint256 a) public {
        if (from != msg.sender) {
            revert("auth");
        } else {
            token.transferFrom(from, to, a);
        }
    }

    // EIP-3156 lender repayment: receiver is trusted after `.onFlashLoan(...)`.
    function okFlashLender(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) public returns (bool) {
        token.transfer(address(receiver), amount);
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), address(this), amount + fee);
        return true;
    }

    function badTupleReassignKillsSafe(address from, address to, uint256 a) public {
        address x = msg.sender;
        address y;
        (x, y) = (from, to);
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badFlashLoanInBranch(
        bool flag,
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) public {
        if (flag) {
            receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        }
        token.transferFrom(address(receiver), address(this), amount + fee); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badFlashLoanReceiverReassigned(
        IERC3156FlashBorrower receiver,
        IERC3156FlashBorrower untrusted,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) public {
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        receiver = untrusted;
        token.transferFrom(address(receiver), address(this), amount + fee); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badFakeFlashLoan(
        IFakeFlashBorrower fake,
        address to,
        uint256 a,
        bytes calldata data
    ) public {
        fake.onFlashLoan(data);
        token.transferFrom(address(fake), to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Pull-back token differs from the one passed to the hook.
    function badFlashLoanWrongToken(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) public {
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        other.transferFrom(address(receiver), address(this), amount + fee); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Pull-back recipient isn't the lender.
    function badFlashLoanWrongRecipient(
        IERC3156FlashBorrower receiver,
        address attacker,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) public {
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), attacker, amount + fee); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Pull-back amount isn't `amount + fee`.
    function badFlashLoanWrongAmount(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        uint256 other,
        bytes calldata data
    ) public {
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), address(this), other); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Second pull-back after the obligation has been consumed.
    function badFlashLoanDoublePull(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) public {
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), address(this), amount + fee);
        token.transferFrom(address(receiver), address(this), amount + fee); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // `fee + amount` (commuted) still matches.
    function okFlashLoanCommutativeAmount(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) public {
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), address(this), fee + amount);
    }

    function badNamedMember(address from, address to, uint256 a) public {
        token.transferFrom({from: from, to: to, amount: a}); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badNamedLibrary(address from, address to, uint256 a) public {
        SafeERC20.safeTransferFrom({token: token, from: from, to: to, value: a}); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function okNamedMemberSender(address to, uint256 a) public {
        token.transferFrom({from: msg.sender, to: to, amount: a});
    }

    function okNamedLibrarySelf(address to, uint256 a) public {
        SafeERC20.safeTransferFrom({token: token, from: address(this), to: to, value: a});
    }

    function okNamedPermit(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit({
            owner: from,
            spender: address(this),
            value: a,
            deadline: deadline,
            v: v,
            r: r,
            s: s
        });
        token.transferFrom({from: from, to: to, amount: a});
    }

    // `delete` clears a prior safe-fact.
    function badDeleteKillsSafe(address from, address to, uint256 a) public {
        address x = msg.sender;
        delete x;
        x = from;
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Flash-loan call on the RHS of `&&` may not execute; its repayment must not leak.
    function badFlashLoanShortCircuit(
        bool flag,
        IERC3156FlashBorrower receiver,
        bytes32 MAGIC,
        uint256 amount,
        uint256 fee,
        bytes calldata data
    ) public returns (bool) {
        bool ok = flag
            && receiver.onFlashLoan(msg.sender, address(token), amount, fee, data) == MAGIC;
        token.transferFrom(address(receiver), address(this), amount + fee); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
        return ok;
    }

    // `do-while` body runs at least once — facts established inside flow out.
    function okDoWhileEstablishesSafe(address from, address to, uint256 a) public {
        address x = from;
        do {
            x = msg.sender;
        } while (false);
        token.transferFrom(x, to, a);
    }

    // `break` may skip the safe assignment.
    function badDoWhileBreakSkipsSafe(address from, address to, uint256 a, bool flag) public {
        address x = from;
        do {
            if (flag) break;
            x = msg.sender;
        } while (false);
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badDoWhileBreakSkipsResafe(address from, address to, uint256 a, bool flag) public {
        address x = msg.sender;
        do {
            x = from;
            if (flag) break;
            x = msg.sender;
        } while (false);
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // `continue` may also skip the safe assignment.
    function badDoWhileContinueSkipsSafe(address from, address to, uint256 a, bool flag) public {
        address x = from;
        do {
            if (flag) continue;
            x = msg.sender;
        } while (false);
        token.transferFrom(x, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    // Nested-loop `break` doesn't target the outer do-while.
    function okDoWhileNestedLoopBreak(address from, address to, uint256 a) public {
        address x = from;
        do {
            for (uint256 i = 0; i < 1; i++) {
                if (i == 0) break;
            }
            x = msg.sender;
        } while (false);
        token.transferFrom(x, to, a);
    }
}

// Struct / array / mapping receivers.
contract ContainerReceivers {
    struct Config {
        IERC20 token;
    }

    Config cfg;
    IERC20[] tokens;
    mapping(uint256 => IERC20) tokenMap;

    function badStructFieldReceiver(address from, address to, uint256 a) public {
        cfg.token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badArrayElementReceiver(address from, address to, uint256 a) public {
        tokens[0].transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function badMappingValueReceiver(uint256 id, address from, address to, uint256 a) public {
        tokenMap[id].transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function okStructFieldSender(address to, uint256 a) public {
        cfg.token.transferFrom(msg.sender, to, a);
    }
}

// Solady-style: first param is `address`, not a contract type.
library SafeTransferLib {
    function safeTransferFrom(address token, address from, address to, uint256 amount) internal {
        token; from; to; amount; // body intentionally elided.
    }
}

contract SoladyCallSites {
    address token;

    function badSolady(address from, address to, uint256 a) public {
        SafeTransferLib.safeTransferFrom(token, from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function okSoladySender(address to, uint256 a) public {
        SafeTransferLib.safeTransferFrom(token, msg.sender, to, a);
    }

    function okSoladySelf(address to, uint256 a) public {
        SafeTransferLib.safeTransferFrom(token, address(this), to, a);
    }
}

contract InternalForwardedPulls {
    address token;

    function okDeposit(address to, uint256 a) public {
        _pull(msg.sender, to, a);
    }

    function okMint(address to, uint256 a) public {
        _pull(payable(msg.sender), to, a);
    }

    function badForward(address from, address to, uint256 a) public {
        _mixedPull(from, to, a);
    }

    function okForward(address to, uint256 a) public {
        _mixedPull(msg.sender, to, a);
    }

    function _pull(address from, address to, uint256 a) internal {
        SafeTransferLib.safeTransferFrom(token, from, to, a);
    }

    function _mixedPull(address from, address to, uint256 a) internal {
        SafeTransferLib.safeTransferFrom(token, from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }
}

// `using ... for address`: 3-arg `safeTransferFrom` member call on an `address`.
contract SoladyUsingForAddress {
    using SafeTransferLib for address;

    address token;

    function badSoladyMember(address from, address to, uint256 a) public {
        token.safeTransferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function okSoladyMemberSender(address to, uint256 a) public {
        token.safeTransferFrom(msg.sender, to, a);
    }

    function okSoladyMemberSelf(address to, uint256 a) public {
        token.safeTransferFrom(address(this), to, a);
    }

    function okSoladyMemberGuarded(address from, address to, uint256 a) public {
        require(from == msg.sender, "auth");
        token.safeTransferFrom(from, to, a);
    }

    function badNamedSolady(address from, address to, uint256 a) public {
        SafeTransferLib.safeTransferFrom({token: token, from: from, to: to, amount: a}); //~WARN: `transferFrom` uses an arbitrary `from`; require it to equal `msg.sender` or `address(this)`
    }

    function okNamedSoladySender(address to, uint256 a) public {
        SafeTransferLib.safeTransferFrom({token: token, from: msg.sender, to: to, amount: a});
    }
}

// ERC721 helper shares the 4-arg shape but is not ERC20.
library SafeERC721 {
    function safeTransferFrom(IERC721 nft, address from, address to, uint256 tokenId) internal {
        nft.safeTransferFrom(from, to, tokenId);
    }
}

contract SafeERC721CallSites {
    IERC721 nft;

    function okSafeErc721ArbitraryFrom(address from, address to, uint256 tokenId) public {
        SafeERC721.safeTransferFrom(nft, from, to, tokenId);
    }
}
