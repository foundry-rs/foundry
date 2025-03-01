# Coins Core

`coins-core` is an abstract description of UTXO transactions. It provides a
collection of traits that provide consistent interfaces to UTXO transaction
construction. Coins's traits ensure that types are consistent across all
steps in the tx construction process, and allow for code reuse when building
transactions on multiple chains (e.g. Bitcoin Mainnet and Bitcoin Testnet).

Many concepts familiar to UTXO chain developers have been genericized.
Transactions are modeled as a collection of `Input`s and `Output`s. Rather than
addresses or scripts, the `Output` trait has an associated
`RecipientIdentifier`. Similarly, rather than an outpoint, the `Input` trait
has an associated `TXOIdentfier`.

Support for other chains may be added by implementing these traits, and
extending the implementations with network-specific functionality. We have
provided an implementation suitable for Bitcoin chains (mainnet, testnet, and
signet) in the `bitcoins` crate.

## Type Layout

#### Ser trait

The `Ser` trait is a simple serialization API using `std::io::{Read, Write}`.
Implementers define the binary serialization format of the type, as well as the
JSON serialization. The transaction type must implement `Ser`, as the provided
`txid` logic assumes access to the `serialize` method.

`Ser` has an associated `Error` type. Most basic types can simply use the
provided `SerError`. However, more complex (de)serialization will want to
implement a custom error type to handle (e.g.) invalid transactions. These
types must be easily instantiated from a `SerError` or an `std::io::Error`.

#### Transaction types

These describe the components of a transaction.
- A `TXOIdentfier` uniquely identifies a transaction output. In Bitcoin, this
    is an outpoint.
- An `Input` describes the input to a transaction. It has an associated
    `TXOIdentfier` that identifies the TXO being consumed, and can be extended
    with ancillary information (e.g. Bitcoin's `nSequence` field).
- A `RecipientIdentifier` identifies the recipient of a new TXO. In Bitcoin,
    these are pubkey scripts.
- An `Output` describes the output to a transaction. It has an associated a
    `RecipientIdentifier` and a `Value` type.
- A `Transaction` is a collection of `Input`s to be consumed, and `Output`s to
    be created. Its associated `Digest` type describes its transaction ID and
    must be produced by its associated `HashWriter`. This allows transactions
    to specify the digest algorithm used to generate their sighash digest and
    their TXID.

#### Encoder types

The encoder translates between human-facing data and protocol-facing data.
Particularly between addresses and `RecipientIdentifier`s
- `Address` is a type that describes the network's address semantics. For
    Bitcoin this is an enum whose members wrap a `String`.
- `AddressEncoder` has associated `Address` and `RecipientIdentifier` types. It
    exposes `encode_address`, `decode_address`, and `string_to_address` it order to
    enable conversion between them.

#### Builder type

The transaction builder provides a convenient interface for constructing
`Transaction` objects. It has associated `Transaction` and `AddressEncoder`
types, and ensures that they use the same `RecipientIdentifier`. This allows us
to provide a simple `pay(value, address)` interface on the builder.

#### Network type

The network type guarantees type consistency across a set of implementing
types, provides a unified interface to accessing them. This is intended to be
the primary entry point for the implementing libraries. It guarantees that the
`Builder`, `AddressEncoder`, and `Transaction` types use the same `Error`, the
same `RecipientIdentifier`, the same `TXOIdentfier`, and the same `Address`
type. It provides passthroughs to the `AddressEncoder`'s associated functions,
and a convenience method for instantiating a new builder.
