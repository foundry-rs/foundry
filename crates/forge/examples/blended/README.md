# Blended Counter

This is a blended Solidity + WASM example project.

## Project Structure

- `src/BlendedCounter.sol` - Main Solidity contract
- `src/power-calculator/` - Rust WASM module for power calculations
- `test/` - Forge tests
- `script/` - Deployment scripts

## Usage

### Build

```shell
gblend build
```

### Test

```shell
gblend test
```

### Deploy

```shell
gblend script script/BlendedCounter.sol:Deploy --rpc-url <your_rpc_url> --private-key <your_private_key>
```

## Documentation

For complete documentation on forge commands, see the [Foundry Book](https://getfoundry.sh/forge/overview).

For Fluent-specific development guides, visit [Fluent Documentation](https://docs.fluent.xyz/gblend/usage).
