# evm-adapters

Abstraction over various EVM implementations via the `Evm` trait. Currently
supported: [Sputnik EVM](https://github.com/rust-blockchain/evm/).

Any implementation of the EVM trait receives [fuzzing support](./src/fuzz.rs)
using the [`proptest`](https://docs.rs/proptest) crate.

## Sputnik's Hooked Executor

In order to implement cheatcodes, we had to hook in EVM execution. This was done
by implementing a `Handler` and overriding the `call` function, in the
[`CheatcodeHandler`](crate::sputnik::cheatcodes::CheatcodeHandler)

## Sputnik's Cached Forking backend

When testing, it is frequently a requirement to be able to fetch live state from
e.g. Ethereum mainnet instead of redeploying the contracts locally yourself.

To assist with that, we provide 2 forking providers:

1. [`ForkMemoryBackend`](crate::sputnik::ForkMemoryBackend): A simple provider
   which calls out to the remote node for any data that it does not have
   locally, and caching the result to avoid unnecessary extra requests
1. [`SharedBackend`](crate::sputnik::cache::SharedBackend): A backend which can
   be cheaply cloned and used in different tests, typically useful for test
   parallelization. Under the hood, it has a background worker which
   deduplicates any outgoing requests from each individual backend, while also
   sharing the return values and cache. This backend not in-use yet.
