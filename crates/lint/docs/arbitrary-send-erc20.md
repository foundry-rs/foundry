# Arbitrary ERC20 send

**Severity**: `High`
**ID**: `arbitrary-send-erc20`

`transferFrom` (and `safeTransferFrom`) move tokens from any address that has
previously approved this contract. If the `from` argument is taken from
user-controlled input without being constrained to `msg.sender` or
`address(this)`, an attacker can pull tokens from any wallet that has an
outstanding allowance to the vulnerable contract.

## What it does

Flags ERC20 transfer calls whose `from` argument is not provably equal to
`msg.sender` or `address(this)`. The lint inspects three call shapes:

- `token.transferFrom(from, to, amount)` (member call on an ERC20-typed
  receiver).
- `token.safeTransferFrom(from, to, amount)` (member call, including
  `using SafeERC20 for IERC20` extensions).
- `Lib.safeTransferFrom(token, from, to, amount)` (library call with the
  4-argument SafeERC20 signature).

Safety is established by:

- Direct use of `msg.sender` / `address(this)` (incl. `address(...)`,
  `payable(...)`, parens, and ternary in which both branches are safe).
- Calls to no-arg helpers whose body is `return X;` where `X` is itself
  statically safe — recognises OpenZeppelin's `_msgSender()` and chains of
  similar wrappers up to a small bounded depth.
- Local variables (and `immutable` / `constant` state variables)
  initialized or last-assigned from a safe expression, or proven equal to
  one via an equality guard.
- Equality guards in `require(...)`, `assert(...)`, and
  `if (... != safe) revert ...;`, including conjunctions thereof.
- Inline equality guards in the prefix of a modifier body (statements
  strictly before its single top-level `_;` placeholder), mapped back to
  the caller's argument variables.
- A prior EIP-2612 `permit(owner, address(this), …)` on the same token
  variable, on the **same execution path** as the sink, with the sink's
  `from` variable matching the permit's `owner`. The record is invalidated
  if either the token variable or the owner variable is reassigned before
  the sink.

Branch joins recognise `return`, custom-error `revert`, the `revert(...)`
builtin, and `assert(false)` / `require(false, ...)` as always-exiting:
facts proven on the surviving branch propagate past the `if`.

## Limitations

The permit suppression is a precision relaxation, not a proof. It matches
the 7-argument `permit(...)` shape on a member call but does **not**:

- verify the receiver type statically declares the EIP-2612 `permit`
  signature (any same-named 7-arg method silences the lint),
- correlate the receiver type between the permit and the sink (only the
  underlying variable is compared),
- model the permit's allowance / value against the sink's amount.

False negatives are possible when a non-EIP-2612 contract exposes a
matching `permit(...)` method. The canonical arbitrary-send pattern (no
preceding permit) is unaffected.

## Why is this bad?

If a user has approved the contract to spend their tokens (e.g. for a swap or
deposit they expect to perform later), an attacker can call a function that
takes an arbitrary `from` and instruct the contract to transfer those tokens
to themselves. This is one of the most common ways funds are drained from
DeFi protocols.

## Example

### Bad

```solidity
function pull(address from, address to, uint256 amount) external {
    token.transferFrom(from, to, amount); // attacker may pass any `from`
}
```

### Good

```solidity
function deposit(uint256 amount) external {
    token.transferFrom(msg.sender, address(this), amount);
}

function pull(address from, address to, uint256 amount) external {
    require(from == msg.sender, "unauthorized");
    token.transferFrom(from, to, amount);
}
```
