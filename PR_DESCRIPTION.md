# feat(fuzz): Improve call_override to detect ETH value transfer reentrancy

## Summary

This PR enhances the `call_override` invariant testing feature to detect reentrancy vulnerabilities that occur during ETH value transfers. Previously, `call_override` only intercepted non-value calls to contracts. Now it also handles the case where a contract sends ETH to an EOA, which is a common reentrancy vector (e.g., the Rari Capital hack).

## Changes

### `crates/evm/fuzz/src/inspector.rs`

The `override_call` function now:

1. **Detects EOA targets**: Checks if the call target has code (contract) or is an EOA
2. **Handles value transfers to EOAs**: When a contract sends ETH to an EOA:
   - Performs the ETH transfer via the journal first
   - Then injects a reentrant callback with value=0
   - This simulates a malicious `receive()` function that reenters
3. **Skips value transfers to contracts**: Lets deposits and other legitimate value transfers execute normally
4. **Skips cheatcode calls**: Avoids interfering with `vm.prank`, `vm.deal`, etc.

### `crates/evm/fuzz/src/strategies/invariants.rs`

Fixed a bug where `override_call_strat` could generate calldata for one contract but target a different address. Now the actual target address and function are correctly paired.

## Testing

Added a new test `invariant_reentrancy_eth_transfer` that demonstrates the feature:

```solidity
contract VulnerableVault {
    function withdraw() external {
        uint256 amount = balances[msg.sender];
        require(amount > 0, "No balance");
        // BUG: Sends ETH before updating state
        (bool success,) = msg.sender.call{value: amount}("");
        require(success, "Transfer failed");
        balances[msg.sender] = 0;  // Too late!
    }
}
```

With `call_override=true`, the fuzzer detects that during `withdraw()`:
1. ETH is sent to the user (EOA)
2. A callback is injected that calls `exploit()` 
3. Since `balances[msg.sender]` hasn't been zeroed yet, the exploit succeeds

## Limitations

The `call_override` feature works best for detecting simple reentrancy patterns where:
- A single callback during an external call can trigger the exploit
- The callback doesn't require specific function parameters

For complex exploits like the Rari Capital hack, which require:
- Specific sequences (mint → borrow → repay during borrow's ETH transfer)
- Specific callback parameters
- Low probability of random discovery (~27% per call)

Manual reentrancy handlers (like the original `TestAccount` in the Rari test) remain more effective as they can be configured to always trigger specific callback patterns.

## Checklist

- [x] Code compiles without errors
- [x] `cargo fmt` passes
- [x] `cargo clippy` passes without warnings
- [x] Tests pass:
  - `invariant_reentrancy` (original, non-value call)
  - `invariant_reentrancy_eth_transfer` (value transfer to EOA)
  - `invariant_reentrancy_eth_transfer_to_contract` (value transfer to contract)
