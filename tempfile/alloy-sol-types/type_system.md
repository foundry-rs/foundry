# Solidity Type Representation

This crate is built around a representation of the Solidity type system.
This doc is a primer for how we chose to represent Solidity types in Rust.

## Why?

The ABI encoding scheme, is tailored to the EVM and to Solidity specifically.
Its internals are complex and may not be well-understood by Solidity devs.
However, Solidity devs generally do understand Solidity types. As a result, we
decided the best way to represent ABI coding was as a method on Solidity types.

Rather than `Encoder::encode(data, type)` we felt that `Type::encode(data)` would
be more intuitive and idiomatic in Rust. To achieve this, we give each Solidity
type a concrete Rust type that contains its data. E.g. `bytes32` is `[u8; 32]`.
`uint256` is `U256`, `string` is `String`. This allows programmers to work with
Solidity _types_, but Rust _data_.

Static ABI typing also allows the compiler to do significantly more
optimization on encoding and decoding. Benchmarks are pending, but we expect
this to be one of the fastest implementations for regular encoding/decoding. :)

## Downside

This crate works only with types known at compile-time. For types known only at
runtime (including the eip712 `eth_signTypedData` JSON-RPC request), see the
`alloy-dyn-abi` crate.

### To what extent?

We support types at the interface between Solidity and other systems. These are
types that are commonly ABI encoded/decoded. We do not support types that are
internal-only (e.g. array slices) or Solidity type modifications describing EVM
internals (e.g. `payable` and `memory`) except where they interact with
external systems.

Mappings and storage items are a special case, as public storage items imply
the existence of a getter function. We do not currently support parsing storage
defs into `SolCall` types, but may in the future.

**Support overview:**

- First-class Solidity types
  - All elementary, fixed-size, and non-fixed size
    [ABI types](https://docs.soliditylang.org/en/latest/abi-spec.html#types).
  - EXCEPT
    - [`fixed`](https://docs.soliditylang.org/en/latest/types.html#fixed-point-numbers).
- Compound Solidity types
  - Arrays `T[N]`
  - Dynamic arrays `T[]`
  - Tuples `(T, U, ..)`
- User-defined Types
    - [Structs](https://solidity-by-example.org/structs/) represented as a tuple of the field types.
    - [User-defined value types](https://blog.soliditylang.org/2021/09/27/user-defined-value-types/), encoded transparently.
    - [Enums](https://docs.soliditylang.org/en/latest/types.html#enums) represented as `u8`.
- Externalized Types
    - Function arguments and returns, represented as selector-prefixed tuples.
    - [Errors](https://blog.soliditylang.org/2021/04/21/custom-errors/), represented as selector-prefixed tuples
    - Events, represented as a tuples of topic and data types.

## How?

Solidity has two basic categories: first-class and externalized. The
first-class types are those that can be bound to variables, passed as
arguments, used as fields in structs, stored (on the stack, in memory or in
storage), etc. The externalized types are only used in EVM lifecycle events,
and cannot be generally operated on beyond construction.

The first-class types implement the `SolType` trait. This trait contains
functionality common to all first-class Solidity types. This includes type
name, ABI (de)tokenization and coding, Solidity type checking rules. These
types include structs, enums, UDTs, address, bool, bytes, string, etc.

Externalized types are calls (function arguments and returns), events, and
errors. These are each represented by a trait (`SolCall`, `SolEvent`, and
`SolError`). These types enter or exit the EVM, or pass between callstack
frames, and are not part of normal Solidity computation. However, they are
composed of first-class types, and their ABI coding uses the first-class type's
ABI coding.

### ⚠️ Rough Edge ⚠️

Currently, our representation supports using tuples as struct props. This is
not allowed in Solidity, and future versions of this crate may change the type
system to disallow it.

### Layout

```
- SolError
- SolCall
- SolEvent
- SolType
  ├── SolStruct
  ├── SolEnum
  ├── UDVTs
  ├── bool
  ├── bytesX (1 - 32)
  ├── intX (8 - 256)
  ├── uintX (8 - 256)
  ├── address
  ├── function (same as bytes24)
  ├── bytes
  ├── string
  ├── T[N] (Array)
  ├── T[] (Dynamic Array)
  └── Tuples `(T, U, ..)`
```

### Trait Quick Reference

- `SolType` - provides type name and properties, ABI coding, packed encoding, and EIP-712 encoding.
- `SolValue` - conveniency wrapper for `SolType` that provides the same interface but as methods on the value type itself.
- `SolStruct` - describes struct types, and provides specialized coding methods.
- `SolEnum` - describes enum types as `u8` wrappers, and provides specialized coding methods.
- `SolError` - describes custom Error types with selector, and provides specialized coding methods.
- `SolCall` - describes function **arguments** with selector, and provides specialized coding methods.
  An associated `Return` type describes function returns.
- `SolEvent` - describes Event types with topics and data, and provides specialized coding methods.

## Implementing these traits

Well, don't.

Due to the weirdness of Solidity and the sensitivity of the ABI coder to minor
issues, we do not recommend manually implementing these traits. Instead, most
users will want to use the `sol!` macro to auto-generate types and structs from
Solidity snippets at compile time.

## Using these traits

Users will typically want to interact with `SolValue`. When using errors,
events, or calls, users will want to import the relevant trait, and use the
specialized coding methods.
