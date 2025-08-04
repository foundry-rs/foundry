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
gblend script script/BlendedCounter.sol:BlendedCounterScript --rpc-url <your_rpc_url> --private-key <your_private_key>
```
