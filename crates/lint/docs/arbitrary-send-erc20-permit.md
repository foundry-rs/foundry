# Arbitrary `from` in `transferFrom` used with `permit`

**Severity**: `High`
**ID**: `arbitrary-send-erc20-permit`

Flags `transferFrom` / `safeTransferFrom` calls whose `from` argument is not provably
`msg.sender` (or `address(this)`) when the function also calls
`token.permit(owner, address(this), …)` for the same token and owner beforehand.

## What it does

Detects, within a single function, the combination of:

1. A preceding `permit(owner, spender, value, deadline, v, r, s)` on `token` with
   `spender == address(this)`, and
2. A subsequent ERC20-style transfer of the same token from the same `owner`, where
   `owner` cannot be proven equal to `msg.sender` or `address(this)`.

Both common sink shapes are recognised:

- Member calls: `token.transferFrom(owner, to, amount)` and
  `token.safeTransferFrom(owner, to, amount)`.
- Library calls: `Lib.safeTransferFrom(token, owner, to, amount)`, including the
  Solady-shaped `using SafeTransferLib for address` form.

The lint does **not** require the transfer `amount` to equal the permit `value`,
nor does it inspect deadlines or signatures — the dangerous combination is the
permit-then-arbitrary-transferFrom pattern itself, not the specific amounts.

Matching EIP-3156 flash-loan repayments (`onFlashLoan` followed by a pull-back of
`amount + fee`) are excluded.

### Scope

The check is intraprocedural. It flags one permit-then-`transferFrom` flow inside a
single function body and correlates the token, owner, and spender by the underlying
variable, with the following normalisations applied to both sides of the correlation:

- elementary type casts (`address(x)`), interface / contract casts (`IERC20(rawToken)`),
  `payable(...)` wraps, and parentheses are stripped;
- local var-to-var copies (`IERC20 t = token; ...`, `address from2 = from; ...`) are
  tracked as aliases, so the permit and the sink still correlate when one side is a copy;
- local aliases of `address(this)` (e.g. `address self = address(this); permit(..., self, ...)`)
  and no-arg helpers whose body is `return address(this);` are recognised as the permit
  spender;
- the `using SafeTransferLib for address` member form is treated as a sink.

Dead code after a top-level `return` / `revert` is skipped — including function bodies
whose modifier prefix definitely exits before `_;`. Inline
`// forge-lint: disable-next-line(arbitrary-send-erc20-permit)` suppresses a single sink.

Additionally supported correlations:

- struct-field token receivers (`cfg.token.permit(...)` then `cfg.token.transferFrom(...)`)
  match via a `(base var, field name)` key;
- the library wrapper `SafeERC20.safePermit(token, ...)` is treated as an EIP-2612
  permit on the `token` argument when the receiver is a library;
- immutable / constant state vars proven equal to `address(this)` or `msg.sender`
  by their declaration initializer or constructor body are recognised as such at
  the start of every function;
- internal calls to functions of the same contract drop facts about every state
  variable the callee (or one nested level of internal callees) assigns to, so a
  prior permit is no longer trusted after the receiver may have been swapped.

Patterns the check still does **not** classify as the permit-variant (the
underlying call may still be reported by `arbitrary-send-erc20` when the sink
itself is unguarded) include permits issued inside a called helper / modifier /
parent contract.

Permits inside `for` / `while` loop bodies do **not** establish facts visible after
the loop (the analyzer treats their execution count as possibly zero), so a
`transferFrom` placed after the loop is not classified as the permit variant. A
`transferFrom` inside the same iteration as the permit is still flagged.

## Why is this bad?

A `permit` followed by `transferFrom` is the textbook EIP-2612 flow, so it looks
safe. It is **not** safe when the token does not actually implement `permit` but
has a fallback function (the canonical example is WETH). On such tokens:

- `permit(...)` is forwarded to the fallback and silently succeeds without
  authorizing anything.
- Any pre-existing allowance from another user to this contract can then be drained
  by anyone, because the contract trusts the (no-op) permit and forwards the
  attacker-supplied `from` straight into `transferFrom`.

The recommendation is to pin the supported token(s) at deploy time and verify they
implement `permit` correctly, or to require `from == msg.sender` so that, even if
the permit silently no-ops, only the caller's own balance is at risk.

If your code separately proves that `permit` succeeded (for example by reading
`token.nonces(owner)` before and after and reverting on no change) or restricts the
sink to a vetted token allowlist, review the finding and suppress with
`// forge-lint: disable-next-line(arbitrary-send-erc20-permit)`.

## Example

### Bad

```solidity
function pullWithPermit(
    address from,
    address to,
    uint256 value,
    uint256 deadline,
    uint8 v,
    bytes32 r,
    bytes32 s
) external {
    token.permit(from, address(this), value, deadline, v, r, s);
    token.transferFrom(from, to, value); // arbitrary-send-erc20-permit
}
```

### Good

```solidity
function pullWithPermit(
    uint256 value,
    uint256 deadline,
    uint8 v,
    bytes32 r,
    bytes32 s
) external {
    // `from` is implicitly the caller — permit + transferFrom only touch the
    // caller's own balance, even if `token` is a non-permit token with a fallback.
    token.permit(msg.sender, address(this), value, deadline, v, r, s);
    token.transferFrom(msg.sender, address(this), value);
}
```
