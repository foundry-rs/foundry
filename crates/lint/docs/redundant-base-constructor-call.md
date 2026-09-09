# Redundant base-constructor call

**Severity**: `Info`
**ID**: `redundant-base-constructor-call`

## What it does

For every base contract listed in a contract's inheritance specifier or invoked from a derived
constructor's header, the lint reports the empty `()` when the base does not require any
arguments.

## Why restrict this?

Writing `A()` suggests an explicit call, but if `A` has no constructor or a zero-parameter
constructor, the parentheses are redundant noise that obscure the real inheritance shape.

A project may retain explicit empty calls to make constructor initialization visually consistent
across its inheritance hierarchy. Suppress the lint when that notation is intentional.

## Example

```solidity
contract A {}
contract B { constructor() {} }

contract C is A() {}                 // A has no constructor
contract D is B { constructor() B() {} } // B's constructor takes no arguments
```

Use instead:

```solidity
contract A {}
contract B { constructor() {} }

contract C is A {}
contract D is B { constructor() {} }
```
