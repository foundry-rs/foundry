# gblend

A Foundry forge wrapper optimized for Fluent Network and WASM smart contract development.

## Installation

### From crates.io

```bash
cargo install gblend
```

### From source

```bash
git clone https://github.com/fluentlabs-xyz/gblend.git
cd gblend
cargo install --path gblend
```

## Motivation

**gblend** is a wrapper around Foundry's forge that uses Fluent's version of REVM and is designed to work with Wasm
smart contracts. It enables you to:

- **Compile Rust smart contracts** for WASM execution
- **Deploy** contracts to Fluent Network
- **Verify** WASM contracts on-chain
- **Create new projects** using templates from [fluentlabs-xyz/examples](https://github.com/fluentlabs-xyz/examples)

The usage is very similar to regular Foundry forge with some additional features. For example, when verifying WASM
contracts, you need to pass the `--wasm` argument. Otherwise, it works almost identically to forge.

## Basic Commands

### Project Management

```bash
# Create a new project using Fluent examples
gblend init my-project

# Create project with specific template
gblend init my-project --template <template-name>

# Build your contracts
gblend build

# Clean build artifacts
gblend clean
```

### Testing

```bash
# Run tests
gblend test

# Run specific test
gblend test --match-test testMyFunction

# Run tests with gas reporting
gblend test --gas-report
```

### Deployment

```bash
# Deploy a contract
# Deploy a Solidity contract
gblend create src/Counter.sol:Counter --rpc-url <rpc-url> --private-key <key> --broadcast --constructor-args <args>

# Deploy a WASM contract with verification
gblend create PowerCalculator.wasm --rpc-url <rpc-url> --private-key <key> --broadcast --verify --verifier blockscout --verifier-url <verifier-url> --wasm

# Deploy using a script
gblend script script/Counter.s.sol:Deploy --rpc-url <rpc-url> --private-key <key> --broadcast
```

### Verification

```bash
# Verify a regular Solidity contract
gblend verify-contract <address> <ContractName> --verifier blockscout --verifier-url <verifier-url>

# Verify a WASM contract (included in deployment command above)
gblend create MyContract.wasm --verify --verifier blockscout --verifier-url <verifier-url> --wasm

# Verify with constructor arguments
gblend verify-contract <address> <ContractName> --constructor-args <args> --verifier blockscout --verifier-url <verifier-url>
```

## Configuration

gblend uses the same configuration system as Foundry forge. Create a `foundry.toml` file in your project root:

```toml
[profile.default]
src = "src"
out = "out"
libs = ["lib"]
optimizer = true
optimizer_runs = 200

# Fluent Network configuration
[rpc_endpoints]
fluent = <rpc-url>

```


## Examples

### Creating a Rust Contract Project

```bash
# Initialize with Fluent Rust contract template
gblend init counter --template counter

cd counter

# Build the Rust contract
gblend build

# Test the contract
gblend test

# Deploy to Fluent testnet
gblend script script/Counter.s.sol:Deploy --rpc-url <rpc-url> --private-key <private-key> --broadcast
```

### Working with WASM Contracts


```bash
# Deploy a pre-compiled WASM contract
gblend create PowerCalculator.wasm --rpc-url <rpc-url> --private-key <private-key> --broadcast

# Verify WASM contract
gblend verify-contract \
    --rpc-url <rpc-url> \
    --verifier blockscout \
    --verifier-url <verifier-url> \
    --wasm \
    0x123... PowerCalculator.wasm
```

## Differences from Standard Forge

- **WASM Support**: Native support for WASM contract compilation and deployment
- **Fluent Templates**: Access to Fluent-specific project templates
- **Enhanced Verification**: `--wasm` flag for verifying WASM contracts
- **Custom REVM**: Support for the fluentbase REVM implementation.

## Documentation

For complete documentation on forge commands, see the [Foundry Book](https://book.getfoundry.sh/).

For Fluent-specific development guides, visit [Fluent Documentation](https://docs.fluent.xyz).

## Contributing

Contributions are welcome! Please see
our [contributing guidelines](https://github.com/fluentlabs-xyz/gblend/blob/main/CONTRIBUTING.md).

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

**Note**: gblend is built on top of Foundry forge and maintains full compatibility with existing forge projects while
adding Fluent Network and WASM-specific enhancements.