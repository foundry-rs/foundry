# Foundry EVM networks

This crate owns Foundry's runtime network-family selection and the network features that can be
shared across Forge, Cast, Anvil, Chisel, and the EVM backend. `NetworkConfigs` represents the active
execution profile; optional Cargo features only make additional profiles available to a binary.

The crate does not instantiate an EVM. Concrete Alloy network and EVM factory types are associated
through `FoundryEvmNetwork` in `foundry-evm-core`.

See the [custom EVM integration guide](../../../docs/dev/networks.md) for ownership boundaries,
state-lifecycle requirements, tool coverage, and CI expectations. API details are published in the
[workspace Rustdoc](https://foundry-rs.github.io/foundry/foundry_evm_networks/).
