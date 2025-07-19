## Foundry

**Foundry is a blazing fast, portable and modular toolkit for Ethereum application development written in Rust.**

Foundry consists of:

- **Forge**: Ethereum testing framework (like Truffle, Hardhat and DappTools).
- **Cast**: Swiss army knife for interacting with EVM smart contracts, sending transactions and getting chain data.
- **Anvil**: Local Ethereum node, akin to Ganache, Hardhat Network.
- **Chisel**: Fast, utilitarian, and verbose solidity REPL.

## Documentation

<https://book.getfoundry.sh/>

## Usage

### Build

```shell
gblend-forge build
```

### Test

```shell
gblend-forge test
```

### Format

```shell
gblend-forge fmt
```

### Gas Snapshots

```shell
gblend-forge snapshot
```

### Anvil

```shell
gblend-anvil
```

### Deploy

```shell
gblend-forge script script/Counter.s.sol:CounterScript --rpc-url <your_rpc_url> --private-key <your_private_key>
```

### Cast

```shell
gblend-cast <subcommand>
```

### Help

```shell
gblend-forge --help
gblend-anvil --help
gblend-cast --help
```
