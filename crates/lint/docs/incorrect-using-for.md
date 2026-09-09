# Incorrect using-for

**Severity**: `Info`
**ID**: `incorrect-using-for`

Flags `using L for T` directives whose library has no function applicable to the type: they attach nothing.

## What it does

Reports `using L for T` when library `L` has no non-private function whose first parameter
accepts `T`, including through an implicit conversion.

`using L for *` and `using {f} for T` are excluded.

## Why is this bad?

A directive that attaches nothing is dead code, and usually a typo: the wrong library, or the wrong type. The compiler accepts it silently, so the mistake surfaces later as a confusing `Member "f" not found` error at the call site, or never surfaces at all.

## Example

```solidity
library CounterLib {
    function increment(uint256 v) internal pure returns (uint256) {
        return v + 1;
    }
}

contract C {
    // no function of CounterLib takes an address
    using CounterLib for address;
}
```

Use instead:

```solidity
contract C {
    using CounterLib for uint256;

    uint256 internal counter;

    function bump() internal view returns (uint256) {
        return counter.increment();
    }
}
```
