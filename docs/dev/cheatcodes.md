# Cheat codes

foundry's EVM support is mainly dedicated to testing and exploration, it features a set of cheat codes which can
manipulate the environment in which the execution is run.

Most of the time, simply testing your smart contracts outputs isn't enough. To manipulate the state of the EVM, as well
as test for specific reverts and events, Foundry is shipped with a set of cheat codes.

## `revm` `Inspector`

To understand how cheat codes are implemented, we first need to look
at [`revm::Inspector`](https://docs.rs/revm/latest/revm/trait.Inspector.html), a trait that provides a set of event
hooks to be notified at certain stages of EVM execution.

For example [`Inspector::call`](https://docs.rs/revm/latest/revm/trait.Inspector.html#method.call) is called wen the EVM is about to execute a call:

```rust
 fn call(
    &mut self,
    _data: &mut EVMData<'_, DB>,
    _inputs: &mut CallInputs,
    _is_static: bool
) -> (InstructionResult, Gas, Bytes) { ... }
```

## [Foundry Inspectors](../../evm/src/executor/inspector)

the `evm` crate has a variety of inspectors for different use cases, such as

-   coverage
-   tracing
-   debugger
-   cheat codes + logging

## [Cheat code Inspector](../../evm/src/executor/inspector/cheatcodes)

The concept of cheat codes and cheat code inspector is very simple.

In solidity cheat codes are calls to a specific address, the cheat code handler address:

`address(bytes20(uint160(uint256(keccak256('hevm cheat code')))))`: 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D

which can be initialized like `Cheats constant cheats = Cheats(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);`, when
inheriting from `forge-std/Test.sol` it can be accessed via `vm.<cheatcode>` directly.

Since cheat codes are bound to a constant address, the cheat code inspector listens for that address:

```rust
impl Inspector for Cheatcodes {
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == CHEATCODE_ADDRESS {
            // intercepted cheat code call
            // --snip--
        }
    }
}
```

When a call to a cheat code is intercepted we try to decode the calldata into a known cheat code.

Rust bindings for the cheat code interface are generated
via [ethers-rs](https://github.com/gakonst/ethers-rs/) `abigen!` macro:

```rust
// Bindings for cheatcodes
abigen!(
    HEVM,
    r#"[
            roll(uint256)
            warp(uint256)
            fee(uint256)
            // --snip--
    ]"#);
```

If a call was successfully decoded into the `HEVMCalls` enum that the `abigen!` macro generates, the remaining step is
essentially a large `match` over the decoded `HEVMCalls` which serves as the implementation handler for the cheat code.

## Adding a new cheat code

This process consists of 4 steps:

1. add the function signature to the `abigen!` macro so a new `HEVMCalls` variant is generated
2. implement the cheat code handler
3. add a Solidity test for the cheatcode under [`testdata/cheats`](https://github.com/foundry-rs/foundry/tree/master/testdata/cheats)
4. add the function signature
   to [forge-std Vm interface](https://github.com/foundry-rs/forge-std/blob/master/src/Vm.sol)
