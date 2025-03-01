# alloy-sol-types

Solidity type modeling and [ABI] and [EIP-712] codec implementation.

This crate provides tools for expressing Solidity types in Rust, and for
encoding these representations into ABI blobs suitable for smart contract
processing. In other words, you can represent your smart contract args in
native Rust, easily encode them to pass to a smart contract, and easily
decode the smart contract return values.

We do this by representing Solidity types in rust via the [`SolType`] trait.
This trait maps Solidity types to Rust types via the associated
[`SolType::RustType`].

The ABI encoding and decoding is implemented in the [`abi`] module, see [its
documentation](abi) to learn how it works.

[ABI]: https://docs.soliditylang.org/en/latest/abi-spec.html
[EIP-712]: https://eips.ethereum.org/EIPS/eip-712

```rust
use alloy_sol_types::{sol_data::*, SolType, SolValue};

// Represent a Solidity type in rust
type MySolType = FixedArray<Bool, 2>;

let data = [true, false];
let validate = true;

// SolTypes expose their Solidity name :)
assert_eq!(&MySolType::sol_type_name(), "bool[2]");

// SolTypes are used to transform Rust into ABI blobs, and back.
let encoded: Vec<u8> = MySolType::abi_encode(&data);
let decoded: [bool; 2] = MySolType::abi_decode(&encoded, validate)?;
assert_eq!(data, decoded);

// This is more easily done with the `SolValue` trait:
let encoded: Vec<u8> = data.abi_encode();
let decoded: [bool; 2] = <[bool; 2]>::abi_decode(&encoded, validate)?;
assert_eq!(data, decoded);
# Ok::<_, alloy_sol_types::Error>(())
```

## [`sol!`]

The [`sol!`] procedural macro provides a convenient way to define
custom [`SolType`]s and reference primitive ones. See
[its documentation][sol!] for details on how to use it.

## [`SolStruct`]

The [`SolStruct`] trait primarily provides EIP-712 signing support.

```rust
# use alloy_sol_types::{sol, SolStruct};
# use alloy_primitives::U256;
// `sol!` allows you to define struct types!
// You can just paste Solidity into the macro and it should work :)
sol! {
    struct MyStruct {
        uint256 a;
        bytes32 b;
        address[] c;
    }
}

sol! {
    struct MyStruct2 {
        MyStruct a;
        bytes32 b;
        address[] c;
    }
}

// All structs generated with `sol!` implement `crate::SolType` &
// `crate::SolStruct`. This means you get eip-712 signing for freeeeee
let my_struct = MyStruct {
    a: U256::from(1),
    b: [0; 32].into(),
    c: vec![Default::default()],
};

// The `eip712_domain` macro lets you easily define an EIP-712 domain
// object :)
let my_domain = alloy_sol_types::eip712_domain!(
   name: "MyDomain",
   version: "1",
);

// Because all the hard work is done by the `sol!` macro, EIP-712 is as easy
// as calling `eip712_signing_hash` with your domain
let signing_hash = my_struct.eip712_signing_hash(&my_domain);
```

### [`sol!`] User-defined Value Types

Support for user-defined value types is new! These are currently
implemented as wrapper types. Watch this space for more
features!

```rust
# use alloy_sol_types::{sol, sol_data, SolType};
# use alloy_primitives::U256;
// We also also support Solidity value types
sol! {
    type MyValueType is uint256;
}

// UDTs are encoded as their underlying type
let mvt = MyValueType::from(U256::from(1));
assert_eq!(mvt.abi_encode(), sol_data::Uint::<256>::abi_encode(&U256::from(1)));
```

## Tokenization/Detokenization

The process of converting from a Rust type to a to an abi token is called
"Tokenization". Typical users will not access tokenization directly.
Power users should use the [`SolType::tokenize()`] and
[`SolType::detokenize()`] methods.

When implementing your own [`SolType`], a variety of `From` impls have been
provided on the token structs to aid in converting from Rust data to tokens.

## Encoding/Decoding

The process of converting from a [`Token`] to a serialized ABI blob is
called "Encoding". It is the reciprocal of decoding.

ABI encoding and decoding operates on sequences of tokens.

The [`SolType`] encoding and decoding methods operate on Rust types. We
recommend users use them wherever possible. We do not recommend that users
interact with Tokens, except when implementing their own [`SolType`].

[`Token`]: abi::Token

## Licensing

This crate is an extensive rewrite of the
[ethabi](https://github.com/rust-ethereum/ethabi) crate by the parity team.
That codebase is used under the terms of the **MIT** license. We have preserved
the original license notice in files incorporating `ethabi` code.
