<h1 align="center">ark-serialize</h1>
<p align="center">
    <img src="https://github.com/arkworks-rs/algebra/workflows/CI/badge.svg?branch=master">
    <a href="https://github.com/arkworks-rs/algebra/blob/master/LICENSE-APACHE"><img src="https://img.shields.io/badge/license-APACHE-blue.svg"></a>
    <a href="https://github.com/arkworks-rs/algebra/blob/master/LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
    <a href="https://deps.rs/repo/github/arkworks-rs/algebra"><img src="https://deps.rs/repo/github/arkworks-rs/algebra/status.svg"></a>
</p>

`ark-serialize` defines the `CanonicalSerialize` and `CanonicalDeserialize` traits for serializing and deserializing Rust data structures to bytes efficiently. The interfaces offered by these traits are specialized for serializing cryptographic objects. In particular, they offer special support for compressed representation of elliptic curve elements.
Most types in `arkworks-rs` implement these traits.

## Usage

To use `ark-serialize`, add the following to your `Cargo.toml`:

```toml
ark_serialize = "0.4"
```

If you additionally want to derive implementations of the `CanonicalSerialize` and `CanonicalDeserialize` traits for your own types, you can enable the `derive` feature:

```toml
ark_serialize = { version = "0.4", features = ["derive"] }
```

### Examples

Let us first see how to use `ark-serialize` for existing types:

```rust
// We'll use the BLS12-381 pairing-friendly group for this example.
use ark_test_curves::bls12_381::{G1Projective as G1, G2Projective as G2, G1Affine, G2Affine};
use ark_serialize::{CanonicalSerialize, CanonicalDeserialize};
use ark_std::UniformRand;

let mut rng = ark_std::test_rng();
// Let's sample uniformly random group elements:
let a: G1Affine = G1::rand(&mut rng).into();
let b: G2Affine = G2::rand(&mut rng).into();

// We can serialize with compression...
let mut compressed_bytes = Vec::new();
a.serialize_compressed(&mut compressed_bytes).unwrap();
// ...and without:
let mut uncompressed_bytes = Vec::new();
a.serialize_uncompressed(&mut uncompressed_bytes).unwrap();

// We can reconstruct our points from the compressed serialization...
let a_compressed = G1Affine::deserialize_compressed(&*compressed_bytes).unwrap();

// ... and from the uncompressed one:
let a_uncompressed = G1Affine::deserialize_uncompressed(&*uncompressed_bytes).unwrap();

assert_eq!(a_compressed, a);
assert_eq!(a_uncompressed, a);

// If we trust the origin of the serialization
// (eg: if the serialization was stored on authenticated storage),
// then we can skip some validation checks, which can greatly reduce deserialization time.
let a_uncompressed_unchecked = G1Affine::deserialize_uncompressed_unchecked(&*uncompressed_bytes).unwrap();
let a_compressed_unchecked = G1Affine::deserialize_compressed_unchecked(&*compressed_bytes).unwrap();
assert_eq!(a_uncompressed_unchecked, a);
assert_eq!(a_compressed_unchecked, a);
```

If we want to serialize our own structs, we can derive implementations of the `CanonicalSerialize` and `CanonicalDeserialize` traits if all fields implement these traits. For example:

```rust
use ark_test_curves::bls12_381::{G1Affine, G2Affine};
use ark_serialize::{CanonicalSerialize, CanonicalDeserialize};

#[derive(CanonicalSerialize, CanonicalDeserialize)]
pub struct MyStruct {
    a: G1Affine,
    b: G2Affine,
}
```

We can also implement these traits manually. For example:

```rust
use ark_test_curves::bls12_381::{G1Affine, G2Affine};
use ark_serialize::{CanonicalSerialize, CanonicalDeserialize, Compress, SerializationError, Valid, Validate};
use ark_std::io::{Read, Write};

pub struct MyStruct {
    a: G1Affine,
    b: G2Affine,
}

impl CanonicalSerialize for MyStruct {
    // We only have to implement the `serialize_with_mode` method; the other methods 
    // have default implementations that call the latter.
    //
    // Notice that `serialize_with_mode` takes `mode: Compress` as an argument. This 
    // is used to indicate whether we want to serialize with or without compression.
    fn serialize_with_mode<W: Write>(&self, mut writer: W, mode: Compress) -> Result<(), SerializationError> {
        self.a.serialize_with_mode(&mut writer, mode)?;
        self.b.serialize_with_mode(&mut writer, mode)?;
        Ok(())
    }

    fn serialized_size(&self, mode: Compress) -> usize {
        self.a.serialized_size(mode) + self.b.serialized_size(mode)
    }
}

impl CanonicalDeserialize for MyStruct {
    // We only have to implement the `deserialize_with_mode` method; the other methods 
    // have default implementations that call the latter.
    fn deserialize_with_mode<R: Read>(mut reader: R, compress: Compress, validate: Validate) -> Result<Self, SerializationError> {
        let a = G1Affine::deserialize_with_mode(&mut reader, compress, validate)?;
        let b = G2Affine::deserialize_with_mode(&mut reader, compress, validate)?;
        Ok(Self { a, b })
    }
}

// We additionally have to implement the `Valid` trait for our struct.
// This trait specifies how to perform certain validation checks on deserialized types.
// For example, we can check that the deserialized group elements are in the prime-order subgroup.
impl Valid for MyStruct {
    fn check(&self) -> Result<(), SerializationError> {
        self.a.check()?;
        self.b.check()?;
        Ok(())
    }
}
```
