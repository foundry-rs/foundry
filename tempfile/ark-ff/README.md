<h1 align="center">ark-ff</h1>
<p align="center">
    <img src="https://github.com/arkworks-rs/algebra/workflows/CI/badge.svg?branch=master">
    <a href="https://github.com/arkworks-rs/algebra/blob/master/LICENSE-APACHE"><img src="https://img.shields.io/badge/license-APACHE-blue.svg"></a>
    <a href="https://github.com/arkworks-rs/algebra/blob/master/LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
    <a href="https://deps.rs/repo/github/arkworks-rs/algebra"><img src="https://deps.rs/repo/github/arkworks-rs/algebra/status.svg"></a>
</p>

This crate defines Finite Field traits and useful abstraction models that follow these traits.
Implementations of concrete finite fields for some popular elliptic curves can be found in [`arkworks-rs/curves`](https://github.com/arkworks-rs/curves/README.md) under `arkworks-rs/curves/<your favourite curve>/src/fields/`.

This crate contains two types of traits:

- `Field` traits: These define interfaces for manipulating field elements, such as addition, multiplication, inverses, square roots, and more.
- Field `Config`s: specifies the parameters defining the field in question. For extension fields, it also provides additional functionality required for the field, such as operations involving a (cubic or quadratic) non-residue used for constructing the field (`NONRESIDUE`).

The available field traits are:

- [`Field`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/mod.rs#L66) - Interface for a generic finite field.
- [`FftField`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/mod.rs#L419) - Exposes methods that allow for performing efficient FFTs on field elements.
- [`PrimeField`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/mod.rs#L523) - Field with a prime `p` number of elements, also referred to as `Fp`.

The models implemented are:

- [`Quadratic Extension`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/quadratic_extension.rs)
    - [`QuadExtField`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/quadratic_extension.rs#L140) - Struct representing a quadratic extension field, in this case holding two base field elements
    - [`QuadExtConfig`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/quadratic_extension.rs#L27) - Trait defining the necessary parameters needed to instantiate a Quadratic Extension Field
- [`Cubic Extension`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/cubic_extension.rs)
    - [`CubicExtField`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/cubic_extension.rs#L72) - Struct representing a cubic extension field, holds three base field elements
    - [`CubicExtConfig`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/cubic_extension.rs#L27) - Trait defining the necessary parameters needed to instantiate a Cubic Extension Field
  
The above two models serve as abstractions for constructing the extension fields `Fp^m` directly (i.e. `m` equal 2 or 3) or for creating extension towers to arrive at higher `m`. The latter is done by applying the extensions iteratively, e.g. cubic extension over a quadratic extension field.

- [`Fp2`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/fp2.rs#L103) - Quadratic extension directly on the prime field, i.e. `BaseField == BasePrimeField`
- [`Fp3`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/fp3.rs#L54) - Cubic extension directly on the prime field, i.e. `BaseField == BasePrimeField`
- [`Fp6_2over3`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/fp6_2over3.rs#L48) - Extension tower: quadratic extension on a cubic extension field, i.e. `BaseField = Fp3`, but `BasePrimeField = Fp`.
- [`Fp6_3over2`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/fp6_3over2.rs#L49) - Extension tower, similar to the above except that the towering order is reversed: it's a cubic extension on a quadratic extension field, i.e. `BaseField = Fp2`, but `BasePrimeField = Fp`. Only this latter one is exported by default as `Fp6`.
- [`Fp12_2over3over2`](https://github.com/arkworks-rs/algebra/blob/master/ff/src/fields/models/fp12_2over3over2.rs#L83) - Extension tower: quadratic extension of `Fp6_3over2`, i.e. `BaseField = Fp6`.

## Usage

There are two important traits when working with finite fields: [`Field`],
and [`PrimeField`]. Let's explore these via examples.

### [`Field`]

The [`Field`] trait provides a generic interface for any finite field.
Types implementing [`Field`] support common field operations
such as addition, subtraction, multiplication, and inverses.

```rust
use ark_ff::Field;
// We'll use a field associated with the BLS12-381 pairing-friendly
// group for this example.
use ark_test_curves::bls12_381::Fq2 as F;
// `ark-std` is a utility crate that enables `arkworks` libraries
// to easily support `std` and `no_std` workloads, and also re-exports
// useful crates that should be common across the entire ecosystem, such as `rand`.
use ark_std::{One, UniformRand};

let mut rng = ark_std::test_rng();
// Let's sample uniformly random field elements:
let a = F::rand(&mut rng);
let b = F::rand(&mut rng);

// We can add...
let c = a + b;
// ... subtract ...
let d = a - b;
// ... double elements ...
assert_eq!(c + d, a.double());

// ... multiply ...
let e = c * d;
// ... square elements ...
assert_eq!(e, a.square() - b.square());

// ... and compute inverses ...
assert_eq!(a.inverse().unwrap() * a, F::one()); // have to to unwrap, as `a` could be zero.
```

In some cases, it is useful to be able to compute square roots of field elements
(e.g.: for point compression of elliptic curve elements).
To support this, users can implement the `sqrt`-related methods for their field type. This method
is already implemented for prime fields (see below), and also for quadratic extension fields.

The `sqrt`-related methods can be used as follows:

```rust
use ark_ff::Field;
// As before, we'll use a field associated with the BLS12-381 pairing-friendly
// group for this example.
use ark_test_curves::bls12_381::Fq2 as F;
use ark_std::{One, UniformRand};

let mut rng = ark_std::test_rng();
let a = F::rand(&mut rng);

// We can check if a field element is a square by computing its Legendre symbol...
if a.legendre().is_qr() {
    // ... and if it is, we can compute its square root.
    let b = a.sqrt().unwrap();
    assert_eq!(b.square(), a);
} else {
    // Otherwise, we can check that the square root is `None`.
    assert_eq!(a.sqrt(), None);
}
```

### [`PrimeField`]

If the field is of prime order, then users can choose
to implement the [`PrimeField`] trait for it. This provides access to the following
additional APIs:

```rust
use ark_ff::{Field, PrimeField, FpConfig, BigInteger};
// Now we'll use the prime field underlying the BLS12-381 G1 curve.
use ark_test_curves::bls12_381::Fq as F;
use ark_std::{One, Zero, UniformRand};

let mut rng = ark_std::test_rng();
let a = F::rand(&mut rng);
// We can access the prime modulus associated with `F`:
let modulus = <F as PrimeField>::MODULUS;
assert_eq!(a.pow(&modulus), a);

// We can convert field elements to integers in the range [0, MODULUS - 1]:
let one: num_bigint::BigUint = F::one().into();
assert_eq!(one, num_bigint::BigUint::one());

// We can construct field elements from an arbitrary sequence of bytes:
let n = F::from_le_bytes_mod_order(&modulus.to_bytes_le());
assert_eq!(n, F::zero());
```
