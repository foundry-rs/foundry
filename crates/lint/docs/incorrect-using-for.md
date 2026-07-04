# Incorrect using-for

**Severity**: `Info`
**ID**: `incorrect-using-for`

Flags `using L for T` directives whose library has no function applicable to the type: they attach nothing.

## What it does

Reports a `using ... for` directive, file-level or contract-level, when no function of the named library accepts the target type as its bound first parameter. Attachment follows the type checker, implicit conversions included: a `uint8` binds to a `uint256` parameter, a derived contract to a base parameter, and a storage reference to a memory one. `using L for *` and the braced form `using {f} for T` are out of scope, the latter because the compiler already rejects a function that cannot attach. This mirrors Slither's `incorrect-using-for` detector.

## Why is this bad?

A directive that attaches nothing is dead code, and usually a typo: the wrong library, or the wrong type. The compiler accepts it silently, so the mistake surfaces later as a confusing `Member "f" not found` error at the call site, or never surfaces at all.

## Example

### Bad

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

### Good

```solidity
contract C {
    using CounterLib for uint256;

    uint256 internal counter;

    function bump() internal view returns (uint256) {
        return counter.increment();
    }
}
```
