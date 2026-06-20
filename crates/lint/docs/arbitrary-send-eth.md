# Arbitrary Send ETH

**Severity**: `High`
**ID**: `arbitrary-send-eth`

Detects functions that send ETH to a destination the caller controls. When the
destination of `transfer` / `send` / a `{value: …}` low-level call /
`selfdestruct` is reachable from a function parameter or mutable storage that
any caller can rewrite, an attacker can redirect the contract's funds.

## What it does

For each non-`view` / non-`pure`, non-constructor, non-library function, the
lint flags ETH-sending sinks whose destination is not provably safe and the
function is not caller-restricted. Sinks inside modifier bodies are reported
at the modifier definition.

Sinks recognised:

- `addr.transfer(amount)` / `addr.send(amount)`
- `recv.method{value: x}(...)` (including `recv.call{value: x}(...)` and
  function-pointer `f{value: x}()`)
- `selfdestruct(addr)`
- Known ETH helpers — OpenZeppelin `Address.{sendValue, functionCallWithValue}`
  and Solady `SafeTransferLib.{safeTransferETH, forceSafeTransferETH,
  safeTransferAllETH, forceSafeTransferAllETH, trySafeTransferETH,
  trySafeTransferAllETH, safeMoveETH}`. Both positional and named-arg calls
  are recognised; static `Base.method(...)` is only matched on `library` bases.

A destination is safe when it is `msg.sender` (also through OZ-style
`_msgSender()` helpers), `tx.origin`, `address(this)`, a fixed literal
(including `address(0)`), an `immutable` / `constant` address, a local proven
equal to a safe value on the current path, or a modifier parameter validated
against `msg.sender`. Flow-sensitive facts come from assignments, `require`,
`assert`, `if`/`else` (incl. ternaries), `&&` / `||` of equality checks, and
`address(...)` / `payable(...)` casts.

Caller-restriction is detected for equality guards (`require(msg.sender ==
trusted)`, `if (msg.sender != trusted) revert()`, De-Morgan equivalents) in
modifier prefixes or function bodies. Trusted principals include state
variables, `immutable` / `constant` addresses, fixed address literals, and
zero-arg helpers returning such. `address(this)` is **not** trusted (a
sibling can route arbitrary callers via `this.guarded(userArg)`); state vars
that *may* alias `address(this)` — through any initializer, constructor,
runtime assignment, alias chain, struct/mapping/array slot, ternary, helper
call (positional or named), base-constructor argument, or identity helper —
are likewise rejected.

## Known limitations

- No general inter-procedural taint: a wrapper forwarding an arbitrary
  destination may be missed. Library bodies are skipped; lint call sites.
- Internal/private helpers are linted in isolation; a sink inside one is
  reported even when every caller is protected.
- Code after `_;` in a modifier is analysed without knowing whether each
  callsite is itself caller-restricted.
- Constructors are not analysed.
- Caller-restriction is heuristic and only recognises equality guards.
  Role-library helpers (`_checkRole`, `hasRole`) and parameterized
  `only(who)` modifiers are not modelled.
- Trusted-principal helpers are recognised only for bare-ident zero-arg
  calls whose body is `return expr;`.
- Mutable state vars are accepted as trusted principals: a mutable `owner`
  plus an unprotected `setOwner` is not flagged. Prefer `immutable`.

## Why is this bad?

If an attacker can choose the recipient of ETH transfers they can drain the
contract balance, redirect user funds, or trivially bypass weak access
controls.

## Example

### Bad

```solidity
contract Vault {
    function withdraw(address payable to, uint256 amount) external {
        to.transfer(amount); // attacker passes their own address
    }
}
```

### Good

```solidity
contract Vault {
    address payable public immutable owner;

    constructor(address payable _owner) { owner = _owner; }

    modifier onlyOwner() { require(msg.sender == owner); _; }

    function withdrawTo(address payable to, uint256 amount) external onlyOwner {
        to.transfer(amount);
    }

    function refund(uint256 amount) external {
        payable(msg.sender).transfer(amount);
    }
}
```
