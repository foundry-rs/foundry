# Arbitrary `from` in `transferFrom` used with `permit`

**Severity**: `High`
**ID**: `arbitrary-send-erc20-permit`

## What it does

Flags `transferFrom` and `safeTransferFrom` calls preceded by a `permit` for the same token
and owner in the same function, with this contract as the spender, when `from` is not
constrained to `msg.sender` or `address(this)`. This includes common SafeERC20 and
SafeTransferLib wrappers.

A permit does not make an arbitrary `from` safe, even when its value matches the transfer
amount. Matching EIP-3156 flash-loan repayments are excluded.

Permits issued in a separate helper or modifier are not correlated with the transfer;
such transfers may instead be reported by `arbitrary-send-erc20`.

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
transfer to a vetted token allowlist, review the finding and suppress with
`// forge-lint: disable-next-line(arbitrary-send-erc20-permit)`.

## Example

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

Use instead:

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
