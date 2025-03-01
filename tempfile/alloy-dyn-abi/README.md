# alloy-dyn-abi

Dynamic Solidity type encoder.

Run-time representation of Ethereum's type system with ABI encoding & decoding.

This library provides a runtime encoder/decoder for solidity types. It is
intended to be used when the solidity type is not known at compile time.
This is particularly useful for EIP-712 signing interfaces.

We **strongly** recommend using the [static encoder/decoder][abi] when possible.
The dynamic encoder/decoder is significantly more expensive, especially for
complex types. It is also significantly more error prone, as the mapping
between solidity types and rust types is not enforced by the compiler.

[abi]: https://docs.rs/alloy-sol-types

## Examples

Basic usage:

```rust
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::hex;

// parse a type from a string
// note: eip712 `CustomStruct`s cannot be parsed this way.
let my_type: DynSolType = "uint16[2][]".parse().unwrap();

// decode
let my_data = hex!(
    "0000000000000000000000000000000000000000000000000000000000000020" // offset
    "0000000000000000000000000000000000000000000000000000000000000001" // length
    "0000000000000000000000000000000000000000000000000000000000000002" // .[0][0]
    "0000000000000000000000000000000000000000000000000000000000000003" // .[0][1]
);
let decoded = my_type.abi_decode(&my_data)?;

let expected = DynSolValue::Array(vec![DynSolValue::FixedArray(vec![2u16.into(), 3u16.into()])]);
assert_eq!(decoded, expected);

// roundtrip
let encoded = decoded.abi_encode();
assert_eq!(encoded, my_data);
# Ok::<(), alloy_dyn_abi::Error>(())
```

EIP-712:

```rust,ignore
todo!()
```

## How it works

The dynamic encoder/decoder is implemented as a set of enums that represent
solidity types, solidity values (in rust representation form), and ABI
tokens. Unlike the static encoder, each of these must be instantiated at
runtime. The [`DynSolType`] enum represents a solidity type, and is
equivalent to an enum over types implementing the [`crate::SolType`] trait.
The [`DynSolValue`] enum represents a solidity value, and describes the
rust shapes of possible solidity values. It is similar to, but not
equivalent to an enum over types used as [`crate::SolType::RustType`]. The
[`DynToken`] enum represents an ABI token, and is equivalent to an enum over
the types implementing the [`alloy_sol_types::abi::Token`] trait.

Where the static encoding system encodes the expected type information into
the Rust type system, the dynamic encoder/decoder encodes it as a concrete
instance of [`DynSolType`].

- Detokenizing: `DynSolType` + `DynToken` = `DynSolValue`

Users must manually handle the conversions between [`DynSolValue`] and their
own rust types. We provide several `From` implementations, but they fall
short when dealing with arrays, tuples and structs. We also provide fallible
casts into the contents of each variant.

## `DynToken::decode_populate`

Because the shape of the data is known only at runtime, we cannot
compile-time allocate the memory needed to hold decoded data. Instead, we
pre-allocate a [`DynToken`] with the same shape as the expected type, and
empty values. We then populate the empty values with the decoded data.

This is a significant behavior departure from the static decoder. We do not
recommend using the [`DynToken`] type directly. Instead, we recommend using
the encoding and decoding methods on [`DynSolType`].

## Licensing

This crate is an extensive rewrite of the
[ethabi](https://github.com/rust-ethereum/ethabi) crate by the parity team.
That codebase is used under the terms of the **MIT** license. We have preserved
the original license notice in files incorporating `ethabi` code.
