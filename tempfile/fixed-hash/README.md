# Fixed Hash

Provides macros to construct custom fixed-size hash types.

## Examples

Simple 256 bit (32 bytes) hash type.

```rust
use fixed_hash::construct_fixed_hash;

construct_fixed_hash! {
    /// My 256 bit hash type.
    pub struct H256(32);
}
```

Opt-in to add conversions between differently sized hashes.

```rust
construct_fixed_hash!{ struct H256(32); }
construct_fixed_hash!{ struct H160(20); }
// auto-implement conversions between H256 and H160
impl_fixed_hash_conversions!(H256, H160);
// now use the generated conversions
assert_eq!(H256::from(H160::zero()), H256::zero());
assert_eq!(H160::from(H256::zero()), H160::zero());
```

It is possible to add attributes to your types, for example to make them serializable.

```rust
construct_fixed_hash!{
    /// My serializable hash type.
    #[cfg_attr(feature = "serialize", derive(Serialize, Deserialize))]
    struct H160(20);
}
```

## Features

By default this is an standard library depending crate.  
For a `#[no_std]` environment use it as follows:

```
fixed-hash = { version = "0.3", default-features = false }
```

### Available Features

- `std`: Use the standard library instead of the core library.
	- Using this feature enables the following features
		- `rustc-hex/std`
		- `rand/std`
		- `byteorder/std`
    - Enabled by default.
- `libc`: Use `libc` for implementations of `PartialEq` and `Ord`.
    - Enabled by default.
- `rand`: Provide API based on the `rand` crate.
    - Enabled by default.
- `byteorder`: Provide API based on the `byteorder` crate.
    - Enabled by default.
- `quickcheck`: Provide `quickcheck` implementation for hash types.
    - Disabled by default.
- `api-dummy`: Generate a dummy hash type for API documentation.
    - Enabled by default at `docs.rs`
- `arbitrary`: Allow for creation of a hash from random unstructured input.
    - Disabled by default.
