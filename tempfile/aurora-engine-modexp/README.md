# Aurora `modexp` implementation

## What this crate is

This crate is an efficient implementation of the EVM `modexp` precompile.
This crate exposes a single public function

```rust
pub fn modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8>
```

This function takes the base, exponent and modulus as big-endian encoded bytes and returns the result in big-endian as well.

This crate is meant to be an efficient implementation, using as little memory as possible (for example, it does not copy the exponent slice).
The exponentiation is done using the ["binary method"](https://en.wikipedia.org/wiki/Exponentiation_by_squaring).
The multiplication steps within the exponentiation use ["Montgomery multiplication"](https://en.wikipedia.org/wiki/Montgomery_modular_multiplication).
In the case of even modulus, Montgomery multiplication does not apply directly.
However we can reduce the problem to one involving an odd modulus and one where the modulus is a power of two.
These two sub-problems can be solved efficiently (the former using Montgomery multiplication, the latter the modular arithmetic is trivial on a binary computer),
then the results are combined using the [Chinese remainder theorem](https://en.wikipedia.org/wiki/Chinese_remainder_theorem).

The primary academic references for this implementation are:

1. [Analyzing and Comparing Montgomery Multiplication Algorithms](https://www.microsoft.com/en-us/research/wp-content/uploads/1996/01/j37acmon.pdf)
2. [Montgomery Reduction with Even Modulus](http://www.people.vcu.edu/~jwang3/CMSC691/j34monex.pdf)
3. [A Cryptographic Library for the Motorola DSP56000](https://link.springer.com/content/pdf/10.1007/3-540-46877-3_21.pdf)
4. [The Art of Computer Programming Volume 2](https://www-cs-faculty.stanford.edu/~knuth/taocp.html)

## What this crate is NOT

This crate is not a general purpose big integer library.
If you need anything other than `modexp`, then you should use something like [num-bigint](https://crates.io/crates/num-bigint) or [ibig](https://crates.io/crates/ibig).
