# Optimism Deprecation

**Severity**: `Low`
**ID**: `optimism-deprecation`

Flags usage of Optimism predeploy addresses that were removed in the Bedrock upgrade, and
calls to GasPriceOracle functions that revert unconditionally since the Ecotone upgrade.

## What it does

Detects two categories of deprecated Optimism L2 patterns:

1. **Removed predeploy address literals**: any hex literal matching one of the four predeploys
   that no longer exist on Bedrock-and-later chains:
   - `0x4200000000000000000000000000000000000000`: LegacyMessagePasser
   - `0x4200000000000000000000000000000000000001`: L1MessageSender
   - `0x4200000000000000000000000000000000000002`: DeployerWhitelist
   - `0x4200000000000000000000000000000000000013`: L1BlockNumber

2. **Deprecated GasPriceOracle functions**: calls to `.overhead()`, `.scalar()`, or
   `.getL1GasUsed()` on the GasPriceOracle predeploy
   (`0x420000000000000000000000000000000000000F`). These functions existed pre-Ecotone but
   revert unconditionally on current OP Stack deployments.

## Why is this bad?

Calling removed predeploys will revert every time as there is no code at those addresses.
Calling the deprecated GasPriceOracle functions will also revert, silently breaking any
gas-estimation or fee-calculation logic that depends on them. Both issues are silent failures 
that only surface at runtime on an OP Stack chain.

## Example

### Bad

```solidity
interface IGasPriceOracle {
    function overhead() external view returns (uint256);
    function scalar() external view returns (uint256);
    function getL1GasUsed(bytes memory data) external view returns (uint256);
}

function legacyFee(bytes memory data) external view returns (uint256) {
    // overhead() and scalar() revert post-Ecotone
    uint256 o = IGasPriceOracle(0x420000000000000000000000000000000000000F).overhead();
    uint256 s = IGasPriceOracle(0x420000000000000000000000000000000000000F).scalar();
    return o + s;
}

function passMessage() external {
    // LegacyMessagePasser no longer exists
    address(0x4200000000000000000000000000000000000000).call("");
}
```

### Good

```solidity
interface IGasPriceOracle {
    function getL1Fee(bytes memory data) external view returns (uint256);
    function baseFeeScalar() external view returns (uint32);
    function blobBaseFeeScalar() external view returns (uint32);
}

function ecotoneL1Fee(bytes memory data) external view returns (uint256) {
    // Use the current GasPriceOracle API available post-Ecotone
    return IGasPriceOracle(0x420000000000000000000000000000000000000F).getL1Fee(data);
}
```

## Notes

This lint is only relevant to contracts deployed on OP Stack chains (Optimism, Base, etc.).
On Ethereum mainnet these addresses are either absent or unrelated contracts, so violations
are harmless there but still indicate dead code worth removing.

Deprecated GasPriceOracle function calls are detected even when the predeploy address is
accessed through a local variable alias (e.g. `IGasPriceOracle gpo = IGasPriceOracle(0x...000F); gpo.overhead()`).
Local variable aliases initialised directly from a literal are tracked; state variables and
reassigned locals are not.
