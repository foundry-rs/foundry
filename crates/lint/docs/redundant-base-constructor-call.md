# Redundant base-constructor call

**Severity**: `Info`
**ID**: `redundant-base-constructor-call`

Flags an explicit empty base-constructor specifier (e.g. `is A()` or `constructor() A() {}`)
when the base contract either has no constructor or has a constructor that takes no arguments.
The empty `()` adds no information and can be removed.

## What it does

For every base contract listed in a contract's inheritance specifier or invoked from a derived
constructor's header, the lint reports the empty `()` when the base does not require any
arguments.

## Why is this bad?

Writing `A()` suggests an explicit call, but if `A` has no constructor or a zero-parameter
constructor, the parentheses are redundant noise that obscure the real inheritance shape.

## Example

### Bad

```solidity
contract A {}
contract B { constructor() {} }

contract C is A() {}                 // A has no constructor
contract D is B { constructor() B() {} } // B's constructor takes no arguments
```

### Good

```solidity
contract A {}
contract B { constructor() {} }

contract C is A {}
contract D is B { constructor() {} }
```
