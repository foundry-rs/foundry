//@compile-flags: --only-lint arbitrary-send-erc20-permit

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
    // Same name as ERC20 but a different ABI; permit-variant must not flag.
    function transferFrom(address from, address to, uint256 tokenId) external;
    function safeTransferFrom(address from, address to, uint256 tokenId) external;
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

library SafeERC20 {
    function safeTransferFrom(
        IERC20 token,
        address from,
        address to,
        uint256 value
    ) internal {
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

// Solady-shaped library that takes the token as a raw `address`. Enables the
// `using ... for address` sink branch in the analyzer.
library SafeTransferLib {
    function safeTransferFrom(address token, address from, address to, uint256 amount) internal {
        // Body intentionally trivial; matched structurally by signature.
        (token, from, to, amount);
    }
}

struct Config {
    IERC20 token;
}

contract ArbitrarySendErc20Permit {
    using SafeERC20 for IERC20;
    using SafeTransferLib for address;

    IERC20 public token;
    IERC20 public other;
    IERC721 public nft;
    address public owner;
    address public immutable trustedOwner;
    address public immutable vault;
    Config public cfg;

    constructor(address _trustedOwner) {
        trustedOwner = _trustedOwner;
        vault = address(this);
    }

    function _msgSender() internal view returns (address) {
        return msg.sender;
    }

    function _self() internal view returns (address) {
        return address(this);
    }

    modifier onlySelf(address f) {
        require(f == msg.sender, "auth");
        _;
    }

    modifier onlyContract(address spender) {
        require(spender == address(this), "spender");
        _;
    }

    // Rewrites its param: the local fact must NOT be hoisted to the caller's var.
    modifier rewriteSpender(address spender) {
        spender = address(this);
        _;
    }

    // Prefix definitely exits — the wrapped function body is unreachable.
    modifier neverRunsBody() {
        return;
        _;
    }

    // -- POSITIVE CASES (should warn with `arbitrary-send-erc20-permit`) --

    // Plain permit + arbitrary-`from` transferFrom on the same `(token, owner)`.
    function badPermitPlain(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Same as `badPermitPlain` but the sink is `safeTransferFrom` (member form).
    function badPermitSafeMember(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        token.safeTransferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Library-form sink: `SafeERC20.safeTransferFrom(token, from, to, a)` still triggers
    // when a permit on the same `(token, from)` was recorded earlier.
    function badPermitLibrary(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        SafeERC20.safeTransferFrom(token, from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Named-args form for both permit and transferFrom.
    function badPermitNamedArgs(
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
        token.transferFrom({from: from, to: to, amount: a}); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Both `then` and `else` permit + sink: each branch sink emits independently.
    function badPermitBothBranches(
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
            token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
        } else {
            token.permit(from, address(this), a, deadline, v, r, s);
            token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
        }
    }

    // Permit set on every path before a post-join sink: join intersection keeps the
    // permit record, so the single sink emits once.
    function badPermitBothBranchesThenSink(
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
        } else {
            token.permit(from, address(this), a, deadline, v, r, s);
        }
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Permit + sink inside a single branch.
    function badPermitInBranch(
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
            token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
        }
    }

    // Permit + sink across an `unchecked` block.
    function badPermitInUncheckedBlock(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        unchecked {
            token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
        }
    }

    // Interface-cast token: `IERC20(rawToken).permit(...)` then
    // `IERC20(rawToken).transferFrom(...)` correlates via the underlying address var.
    function badPermitInterfaceCast(
        address rawToken,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        IERC20(rawToken).permit(from, address(this), a, deadline, v, r, s);
        IERC20(rawToken).transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Solady-shaped library: token passed as raw `address`; permit recorded under the
    // same underlying var via the interface cast.
    function badPermitSoladyAddressToken(
        address rawToken,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        IERC20(rawToken).permit(from, address(this), a, deadline, v, r, s);
        SafeTransferLib.safeTransferFrom(rawToken, from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // `using SafeTransferLib for address;` member form: `rawToken.safeTransferFrom(...)`.
    function badPermitSoladyUsingForAddress(
        address rawToken,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        IERC20(rawToken).permit(from, address(this), a, deadline, v, r, s);
        rawToken.safeTransferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // `do { permit } while (false)` establishes the permit on the only path through the
    // loop body (no `break`/`continue`).
    function badPermitDoWhileEstablishesPermit(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        do {
            token.permit(from, address(this), a, deadline, v, r, s);
        } while (false);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Try-call succeeds; sink inside the success clause inherits the permit from the
    // pre-try state. Falls within the try body's isolation scope: established facts from
    // before the try are visible inside the clause.
    function badPermitBeforeTrySinkInClause(
        IERC20 t,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        try t.transfer(to, a) returns (bool) {
            token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
        } catch {}
    }

    // Spender alias: `address self = address(this)` then `permit(owner, self, …)`.
    // The local copy is tracked in `self_vars`, so the permit IS recorded.
    function badPermitSpenderAlias(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address self = address(this);
        token.permit(from, self, a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // `payable(address(this))` as spender — also recognised.
    function badPermitPayableSelfSpender(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, payable(address(this)), a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Two distinct permits on two distinct tokens, sinks in reverse order. Permits don't
    // consume on use, so both transferFroms must warn.
    function badTwoPermitsTwoSinksDifferentOrder(
        address from1,
        address from2,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from1, address(this), a, deadline, v, r, s);
        other.permit(from2, address(this), a, deadline, v, r, s);
        other.transferFrom(from2, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
        token.transferFrom(from1, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Modifier-hoisted `spender == address(this)` guard. The modifier proves the caller
    // arg is a self alias; the permit IS recorded and the post-permit sink flags the
    // permit variant.
    function badPermitModifierSpenderGuard(
        address from,
        address spender,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public onlyContract(spender) {
        token.permit(from, spender, a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Equality guard `spender == address(this)` makes `spender` a self alias for the
    // permit call, so the record IS stored and the subsequent sink flags the permit
    // variant.
    function badPermitEqualityGuardSpender(
        address from,
        address spender,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        require(spender == address(this), "spender");
        token.permit(from, spender, a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // After consuming an EIP-3156 repayment, the *same-shape* second pullback no longer
    // qualifies as a repayment, but the permit record survives → permit-variant fires.
    function badPermitPlusFlashDoublePull(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(address(receiver), address(this), amount + fee, deadline, v, r, s);
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), address(this), amount + fee);
        token.transferFrom(address(receiver), address(this), amount + fee); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Permit + double flash-loan: two identical `onFlashLoan` calls each license one
    // repayment, so two same-shape transferFroms both consume; neither warns. (Negative.)
    function okPermitDoubleFlashRepayment(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(address(receiver), address(this), amount + fee, deadline, v, r, s);
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), address(this), amount + fee);
        token.transferFrom(address(receiver), address(this), amount + fee);
    }

    // Repayment recipient is a local self alias (`address self = address(this)`); the
    // sink's `to` is the alias, which `is_self_expr` recognises. Repayment is consumed,
    // no warning.
    function okPermitFlashRepaymentSelfAlias(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address self = address(this);
        token.permit(address(receiver), self, amount + fee, deadline, v, r, s);
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), self, amount + fee);
    }

    // Disjunction guard: `from == msg.sender || from == address(this)` proves `from` is
    // safe (both disjuncts establish the same fact); no warning expected.
    function okPermitDisjunctionGuard(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        require(from == msg.sender || from == address(this), "auth");
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // -- NEGATIVE CASES (should NOT warn with `arbitrary-send-erc20-permit`) --

    // Caller is `msg.sender` — even if permit silently no-ops, only the caller's own balance
    // is touched.
    function okPermitMsgSender(
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(msg.sender, address(this), a, deadline, v, r, s);
        token.transferFrom(msg.sender, address(this), a);
    }

    // Explicit equality guard makes `from` provably safe before the sink.
    function okPermitGuarded(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        require(from == msg.sender, "auth");
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // `from = address(this)`: the contract is moving its own tokens.
    function okPermitSelf(
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(address(this), address(this), a, deadline, v, r, s);
        token.transferFrom(address(this), to, a);
    }

    // Modifier-hoisted guard: `f == msg.sender` from the modifier proves `from` safe
    // before the sink runs.
    function okPermitModifierGuarded(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public onlySelf(from) {
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // `_msgSender()` helper resolves to `msg.sender` within the depth budget.
    function okPermitMsgSenderHelper(
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(_msgSender(), address(this), a, deadline, v, r, s);
        token.transferFrom(_msgSender(), address(this), a);
    }

    // Helper used inside an equality guard.
    function okPermitMsgSenderHelperGuard(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        require(from == _msgSender(), "auth");
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // Ternary with both safe branches.
    function okPermitTernaryBothSafe(
        bool flag,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address from = flag ? msg.sender : address(this);
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // Permit spender is *not* `address(this)` — record is not stored, falls through to the
    // plain `arbitrary-send-erc20` lint (out of scope here under `--only-lint`).
    function okPermitWrongSpender(
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
        token.transferFrom(from, to, a);
    }

    // Permit owner differs from sink `from` — no covering record for this `from`.
    function okPermitOwnerMismatch(
        address ownerArg,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(ownerArg, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // Permit on a different token than the sink — no match.
    function okPermitWrongToken(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        other.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // Sink BEFORE the permit: no covering record at the sink.
    function okPermitAfterSink(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.transferFrom(from, to, a);
        token.permit(from, address(this), a, deadline, v, r, s);
    }

    // ERC721's `transferFrom`/`safeTransferFrom` overloads must not match as sinks.
    function okErc721NotAffected(
        address from,
        address to,
        uint256 id,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), id, deadline, v, r, s);
        nft.transferFrom(from, to, id);
        nft.safeTransferFrom(from, to, id);
    }

    // View / pure functions are out of scope for the pass entirely.
    function viewExempt(address from, address to, uint256 a) external view returns (uint256) {
        from; to; a;
        return 0;
    }

    // State-var token reassigned after the permit invalidates the record — the sink falls
    // through to the plain lint, not the permit-variant.
    function okPermitStateTokenReassigned(
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
        token.transferFrom(from, to, a);
    }

    // State-var owner reassigned after the permit invalidates the record.
    function okPermitStateOwnerReassigned(
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        owner = to;
        token.permit(owner, address(this), a, deadline, v, r, s);
        owner = to;
        token.transferFrom(owner, to, a);
    }

    // Permit token reassigned after the permit invalidates the record.
    function okPermitTokenReassigned(
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
        t.transferFrom(from, to, a);
    }

    // Interface-cast raw-token key: reassigning the underlying `rawToken` variable kills
    // the permit record indexed by that var, so the post-reassign sink falls through to
    // the plain lint (filtered out here).
    function okPermitInterfaceCastTokenReassigned(
        address rawToken,
        address otherRawToken,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        IERC20(rawToken).permit(from, address(this), a, deadline, v, r, s);
        rawToken = otherRawToken;
        IERC20(rawToken).transferFrom(from, to, a);
    }

    // Permit owner var reassigned after the permit invalidates the record.
    function okPermitOwnerReassigned(
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
        token.transferFrom(x, to, a);
    }

    // Permit on a side branch must not suppress the fall-through path — and conversely,
    // must not be reported as a permit-variant on the fall-through where no record exists.
    function okPermitInOtherBranch(
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
        // Falls back to the plain `arbitrary-send-erc20` lint (filtered out here).
        token.transferFrom(from, to, a);
    }

    // `do { if (flag) break; permit; } while (false);` may break before permit — the
    // post-loop join drops the permit; sink only triggers the plain lint, not the permit
    // variant.
    function okPermitDoWhileBreakSkipsPermit(
        bool flag,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        do {
            if (flag) break;
            token.permit(from, address(this), a, deadline, v, r, s);
        } while (false);
        token.transferFrom(from, to, a);
    }

    // Permit inside a try clause does NOT leak past the try (visit_isolated semantics);
    // post-try sink falls through to the plain lint, not the permit variant.
    function okPermitInsideTryDoesNotLeak(
        IERC20 t,
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        try t.transfer(to, a) returns (bool) {
            token.permit(from, address(this), a, deadline, v, r, s);
        } catch {}
        token.transferFrom(from, to, a);
    }

    // Catch only runs if the permit reverted; its facts didn't take effect.
    function okPermitRevertedCatchSink(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        try token.permit(from, address(this), a, deadline, v, r, s) {
        } catch {
            token.transferFrom(from, to, a);
        }
    }

    // Success clause inherits the permit fact.
    function badPermitTryCallSuccessSink(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        try token.permit(from, address(this), a, deadline, v, r, s) {
            token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
        } catch {}
    }

    // EIP-3156 repayment matches and suppresses both lints, even when a permit covering
    // `(token, address(receiver))` was recorded beforehand.
    function okPermitPlusFlashRepayment(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(address(receiver), address(this), amount + fee, deadline, v, r, s);
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), address(this), amount + fee);
    }

    // Dead code after a `return` must not trigger either lint (the analyzer stops
    // visiting unreachable statements at the function-body top level).
    function okUnreachableAfterReturn(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        return;
        // dead — no warning expected
        token.transferFrom(from, to, a);
    }

    // Code AFTER a `do { ...; return; } while (false);` is dead too: the do-while
    // definitely exits because the body always returns and there's no break/continue.
    function okUnreachableAfterExitingDoWhile(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        do {
            token.permit(from, address(this), a, deadline, v, r, s);
            return;
        } while (false);
        // dead — no warning expected
        token.transferFrom(from, to, a);
    }

    // Dead code in the do-while fast path (no break/continue) must also be skipped.
    function okUnreachableAfterReturnInDoWhile(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        do {
            token.permit(from, address(this), a, deadline, v, r, s);
            return;
            // dead — no warning expected
            token.transferFrom(from, to, a);
        } while (false);
    }

    // Dead code inside a nested branch also must not trigger (block-level dead-code
    // suppression, not only top-level).
    function okUnreachableAfterReturnInBranch(
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
            return;
            // dead — no warning expected
            token.transferFrom(from, to, a);
        }
    }

    // Branch returns early; the post-`if` fall-through inherits an empty state, so the
    // sink falls through to the plain lint (filtered out here), not the permit variant.
    function okPermitReturningBranchDoesNotLeak(
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
            return;
        }
        token.transferFrom(from, to, a);
    }

    // Modifier that internally rewrites its `spender` parameter must NOT have the
    // resulting local fact hoisted to the caller's `spender`. The caller-side `spender`
    // is unproven, so the permit record is NOT stored and the sink falls through to the
    // plain `arbitrary-send-erc20` lint (out of scope here).
    function okPermitModifierWritesParam(
        address from,
        address spender,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public rewriteSpender(spender) {
        token.permit(from, spender, a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // Tuple swap must be evaluated simultaneously: after `(x, sp) = (sp, x)` the original
    // `sp` (self) has moved to `x`; `sp` now holds the original `x` (unproven). A naive
    // left-to-right model would falsely keep `sp` in `self_vars`. The permit must NOT be
    // recorded → sink falls through to the plain lint (out of scope here).
    function okPermitTupleSwapPreservesFacts(
        address from,
        address spender,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address x = spender;
        address sp = address(this);
        (x, sp) = (sp, x);
        token.permit(from, sp, a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // `from = _msgSender()` propagates safety through the local copy.
    function okPermitFromAssignedMsgSenderHelper(
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address from = _msgSender();
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // `delete token` resets `token`, killing the permit record indexed by it.
    function okPermitDeleteTokenKillsRecord(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        delete token;
        token.transferFrom(from, to, a);
    }

    // Inline `// forge-lint: disable-next-line(arbitrary-send-erc20-permit)` suppresses
    // the warning at the next statement.
    function okInlineDisablePermitLint(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, address(this), a, deadline, v, r, s);
        // forge-lint: disable-next-line(arbitrary-send-erc20-permit)
        token.transferFrom(from, to, a);
    }

    // Every catch always reverts, so post-try retains the permit set by `t.expr`.
    function badPermitTryAllCatchesRevert(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        try token.permit(from, address(this), a, deadline, v, r, s) {} catch {
            revert("permit failed");
        }
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // do-while runs at least once and the only exit is after the permit.
    function badPermitDoWhileBreakAfterPermit(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        do {
            token.permit(from, address(this), a, deadline, v, r, s);
            break;
        } while (true);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Permit's owner is a local alias of the sink's `from`.
    function badPermitOwnerAlias(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address ownerAlias = from;
        token.permit(ownerAlias, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Reverse: sink's `from` is the alias of the permit's owner.
    function badPermitOwnerAliasReverse(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address from2 = from;
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from2, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Sink receiver `t` is a local alias of the permit's token.
    function badPermitTokenAlias(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        IERC20 t = token;
        token.permit(from, address(this), a, deadline, v, r, s);
        t.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // No-arg helper returning `address(this)` is recognised as the permit spender.
    function badPermitHelperSelfSpender(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        token.permit(from, _self(), a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Modifier prefix definitely exits; body is unreachable.
    function okModifierKillsBody(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public neverRunsBody {
        token.permit(from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a);
    }

    // `repayment = amount + fee` satisfies the flash-repayment consumer as a sum-alias.
    function okPermitFlashRepaymentSumAlias(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        uint256 repayment = amount + fee;
        token.permit(address(receiver), address(this), repayment, deadline, v, r, s);
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), address(this), repayment);
    }

    // Reassigning the alias kills it; sink no longer correlates with the permit.
    function okPermitOwnerAliasReassigned(
        address from,
        address other,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) public {
        address ownerAlias = from;
        token.permit(ownerAlias, address(this), a, deadline, v, r, s);
        ownerAlias = other;
        token.transferFrom(ownerAlias, to, a);
    }

    // Permit and sink both go through `cfg.token` (struct-field token receiver).
    function badPermitStructFieldToken(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        cfg.token.permit(from, address(this), a, deadline, v, r, s);
        cfg.token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Library wrapper `SafeERC20.safePermit(token, ...)`; later raw transferFrom on the
    // same token must correlate via the library-form permit match.
    function badPermitSafeWrapperFollowedBySink(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        SafeERC20.safePermit(token, from, address(this), a, deadline, v, r, s);
        token.transferFrom(from, to, a); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Internal call reassigns the token state var; the prior permit no longer covers it.
    function _switchToken() internal {
        token = other;
    }
    function okPermitInternalCallSwitchesToken(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        token.permit(from, address(this), a, deadline, v, r, s);
        _switchToken();
        token.transferFrom(from, to, a);
    }

    // Reassigning a struct field drops permits keyed on `Field(base, name)`.
    function okPermitStructFieldReassigned(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        cfg.token.permit(from, address(this), a, deadline, v, r, s);
        cfg.token = other;
        cfg.token.transferFrom(from, to, a);
    }

    // Every try clause exits; trailing code is unreachable and must not be analyzed.
    function okPermitTryAllClausesExit(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        try token.permit(from, address(this), a, deadline, v, r, s) {
            revert("ok");
        } catch {
            revert("bad");
        }
        token.transferFrom(from, to, a);
    }

    // Arg-eval order: nested sink in args must see facts live before the outer call's
    // state-write side effects are applied.
    function _switchTokenAndReturn(uint256 x) internal returns (uint256) {
        token = other;
        return x;
    }
    function badPermitNestedSinkBeforeStateWrite(
        address from,
        address to,
        uint256 a,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        token.permit(from, address(this), a, deadline, v, r, s);
        _switchTokenAndReturn(token.transferFrom(from, to, a) ? 0 : 0); //~WARN: `transferFrom` uses an arbitrary `from` after `permit`; a non-permit token (e.g. WETH) with a fallback can silently accept the permit and let anyone drain previously-approved tokens
    }

    // Immutable seeded from constructor with `address(this)`; flash-loan repayment
    // to that immutable must be accepted as self.
    function okPermitFlashRepaymentToVault(
        IERC3156FlashBorrower receiver,
        uint256 amount,
        uint256 fee,
        bytes calldata data,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external {
        token.permit(address(receiver), address(this), amount + fee, deadline, v, r, s);
        receiver.onFlashLoan(msg.sender, address(token), amount, fee, data);
        token.transferFrom(address(receiver), vault, amount + fee);
    }
}
