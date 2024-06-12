# Cheatcodes

Foundry's EVM support is mainly dedicated to testing and exploration, it features a set of cheatcodes which can
manipulate the environment in which the execution is run.

Most of the time, simply testing your smart contracts outputs isn't enough. To manipulate the state of the EVM, as well
as test for specific reverts and events, Foundry is shipped with a set of cheatcodes.

## [`revm::Inspector`](https://docs.rs/revm/3.3.0/revm/trait.Inspector.html)

To understand how cheatcodes are implemented, we first need to look at [`revm::Inspector`](https://docs.rs/revm/3.3.0/revm/trait.Inspector.html),
a trait that provides a set of callbacks to be notified at certain stages of EVM execution.

For example, [`Inspector::call`](https://docs.rs/revm/3.3.0/revm/trait.Inspector.html#method.call)
is called when the EVM is about to execute a call:

```rust
fn call(
    &mut self,
    data: &mut EVMData<'_, DB>,
    inputs: &mut CallInputs,
    is_static: bool,
) -> (InstructionResult, Gas, Bytes) { ... }
```

## [Foundry inspectors](../../crates/evm/evm/src/inspectors/)

The [`evm`](../../crates/evm/evm/) crate has a variety of inspectors for different use cases, such as
- coverage
- tracing
- debugger
- logging

## [Cheatcode inspector](../../crates/cheatcodes/src/inspector.rs)

The concept of cheatcodes and cheatcode inspector is very simple.

Cheatcodes are calls to a specific address, the cheatcode handler address, defined as
`address(uint160(uint256(keccak256("hevm cheat code"))))` (`0x7109709ECfa91a80626fF3989D68f67F5b1DD12D`).

In Solidity, this can be initialized as `Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);`,
but generally this is inherited from `forge-std/Test.sol`.

Since cheatcodes are bound to a constant address, the cheatcode inspector listens for that address:

```rust
impl Inspector for Cheatcodes {
    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        is_static: bool,
    ) -> (Return, Gas, Bytes) {
        if call.contract == CHEATCODE_ADDRESS {
            // intercepted cheatcode call
            // --snip--
        }
    }
}
```

When a call to a cheatcode is intercepted we try to decode the calldata into a known cheatcode.

Rust bindings for the cheatcode interface are generated via the [Alloy](https://github.com/alloy-rs) [`sol!`](https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html) macro.

If a call was successfully decoded into the `VmCalls` enum that the `sol!` macro generates, the
last step is a large `match` over the decoded function call structs, which serves as the
implementation handler for the cheatcode. This is also automatically generated, in part, by the
`sol!` macro, through the use of a custom internal derive procedural macro.

## Cheatcodes implementation

All the cheatcodes are defined in a large [`sol!`] macro call in [`cheatcodes/spec/src/vm.rs`]:

```rust
sol! {
#[derive(Cheatcode)]
interface Vm {
    //  ======== Types ========

    /// Error thrown by a cheatcode.
    error CheatcodeError(string message);

    // ...

    // ======== EVM ========

    /// Gets the address for a given private key.
    #[cheatcode(group = Evm, safety = Safe)]
    function addr(uint256 privateKey) external pure returns (address keyAddr);

    /// Gets the nonce of an account.
    #[cheatcode(group = Evm, safety = Safe)]
    function getNonce(address account) external view returns (uint64 nonce);

    // ...
}
}
```

This, combined with the use of an internal [`Cheatcode` derive macro](#cheatcode-derive-macro),
allows us to generate both the Rust definitions and the JSON specification of the cheatcodes.

Cheatcodes are manually implemented through the [`Cheatcode` trait](#cheatcode-trait), which is
called in the [`Cheatcodes` inspector](#cheatcode-inspector) implementation.

### [`sol!`]

Generates the raw Rust bindings for the cheatcodes, as well as lets us specify custom attributes
individually for each item, such as functions and structs, or for entire interfaces.

The way bindings are generated and extra information can be found in the [`sol!`] documentation.

We leverage this macro to apply the [`Cheatcode` derive macro](#cheatcode-derive-macro) on the `Vm` interface.

### [`Cheatcode`](../../crates/macros/src/cheatcodes.rs) derive macro

This is derived once on the `Vm` interface declaration, which recursively applies it to all of the
interface's items, as well as the `sol!`-generated items, such as the `VmCalls` enum.

This macro performs extra checks on functions and structs at compile time to make sure they are
documented and have named parameters, and generates a macro which is later used to implement the
`match { ... }` function that is to be used to dispatch the cheatcode implementations after a call is
decoded.

The latter is what fails compilation when adding a new cheatcode, and is fixed by implementing the
[`Cheatcode` trait](#cheatcode-trait) to the newly-generated function call struct(s).

The `Cheatcode` derive macro also parses the `#[cheatcode(...)]` attributes on functions, which are
used to specify additional properties of the JSON interface.

These are all the attributes that can be specified on cheatcode functions:
- `#[cheatcode(group = <ident>)]`: The group that the cheatcode belongs to. Required.
- `#[cheatcode(status = <ident>)]`: The current status of the cheatcode. E.g. whether it is stable or experimental, etc. Defaults to `Stable`.
- `#[cheatcode(safety = <ident>)]`: Whether the cheatcode is safe to use inside of scripts. E.g. it does not change state in an unexpected way. Defaults to the group's safety if unspecified. If the group is ambiguous, then it must be specified manually.

Multiple attributes can be specified by separating them with commas, e.g. `#[cheatcode(group = "evm", status = "unstable")]`.

### `Cheatcode` trait

This trait defines the interface that all cheatcode implementations must implement.
There are two methods that can be implemented:
- `apply`: implemented when the cheatcode is pure and does not need to access EVM data
- `apply_full`: implemented when the cheatcode needs to access EVM data

Only one of these methods can be implemented.

This trait is implemented manually for each cheatcode in the [`foundry-cheatcodes`](../../crates/cheatcodes/)
crate on the `sol!`-generated function call structs.

### [JSON interface](../../crates/cheatcodes/assets/cheatcodes.json)

The [JSON interface](../../crates/cheatcodes/assets/cheatcodes.json) and [schema](../../crates/cheatcodes/assets/cheatcodes.schema.json)
are automatically generated from the [`sol!` macro call](#sol) by running `cargo cheats`.

The initial execution of this command, following the addition of a new cheat code, will result in an
update to the JSON files, which is expected to fail. This failure is necessary for the CI system to
detect that changes have occurred. Subsequent executions should pass, confirming the successful
update of the files.

### Adding a new cheatcode

1. Add its Solidity definition(s) in [`cheatcodes/spec/src/vm.rs`]. Ensure that all structs and functions are documented, and that all function parameters are named. This will initially fail to compile because of the automatically generated `match { ... }` expression. This is expected, and will be fixed in the next step
2. Implement the cheatcode in [`cheatcodes`] in its category's respective module. Follow the existing implementations as a guide.
3. If a struct, enum, error, or event was added to `Vm`, update [`spec::Cheatcodes::new`]
4. Update the JSON interface by running `cargo cheats` twice. This is expected to fail the first time that this is run after adding a new cheatcode; see [JSON interface](#json-interface)
5. Write an integration test for the cheatcode in [`testdata/cheats/`]

[`sol!`]: https://docs.rs/alloy-sol-macro/latest/alloy_sol_macro/macro.sol.html
[`cheatcodes/spec/src/vm.rs`]: ../../crates/cheatcodes/spec/src/vm.rs
[`cheatcodes`]: ../../crates/cheatcodes/
[`spec::Cheatcodes::new`]: ../../crates/cheatcodes/spec/src/lib.rs#L74
[`testdata/cheats/`]: ../../testdata/default/cheats/
