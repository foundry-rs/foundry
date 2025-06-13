# Polkadot Foundry Supported Forge Commands Documentation with Examples

## Documentation Format and Color Scheme

This documentation is structured to provide a clear overview of the supported `forge` commands. Each command is presented in the following format:

- **Command Name**: The name of the command, colored to indicate its status (**<span style="color: green;">green</span>** for working, **<span style="color: red;">red</span>** for non-working).
- **Command**: The full command syntax with required parameters.
- **Required Parameters**: Parameters that must be provided for the command to execute, as specified in the help files.
- **Example**: A collapsible dropdown containing the complete command with its output or error message, ensuring all relevant details are included.

This format ensures clarity and ease of navigation, with the color scheme providing an immediate visual cue for command reliability.

## Rule of Thumb

- If the command is not listed, it is not supported.
- If the command is listed with a **<span style="color: red;">red</span>** color, it is not supported.
- If the command is listed with a **<span style="color: green;">green</span>** color, it is supported.

## Known Issues

## Forge Commands

### Working Commands

#### ✅ <span style="color: green;">init</span>
- **Command**: `forge init`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge init
  Initializing /test...
  Installing forge-std in /test/lib/forge-std (url: Some("https://github.com/foundry-rs/forge-std"), tag: None)
  Cloning into '/test/lib/forge-std'...
      Installed forge-std v1.9.7
      Initialized forge project
  ```
  </details>

#### ✅ <span style="color: green;">bind</span>
- **Command**: `forge bind`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge bind --resolc
  Compiling 23 files with Resolc v0.2.0, Solc v0.8.30
  installing solc version "0.8.30"
  Successfully installed solc 0.8.30
  Resolc 0.2.0, Solc 0.8.30 finished in 7.22s
  Compiler run successful with warnings:
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdUtils.sol
  Generating bindings for 2 contracts
  Bindings have been generated to /test/out/bindings
  ```
  </details>

#### ✅ <span style="color: green;">bind</span>
- **Command**: `forge bind-json`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge bind-json --resolc
  Compiling 24 files with Resolc v0.2.0, Solc v0.8.30
  installing solc version "0.8.30"
  Successfully installed solc 0.8.30
  Resolc 0.2.0, Solc 0.8.30 finished in 5.23s
  Bindings written to /test/utils/JsonBindings.sol
  ```
  </details>

#### ✅ <span style="color: green;">build</span>
- **Command**: `forge build`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge build --resolc
  Compiling 23 files with Resolc v0.2.0, Solc v0.8.30
  installing solc version "0.8.30"
  Successfully installed solc 0.8.30
  Resolc 0.2.0, Solc 0.8.30 finished in 7.22s
  Compiler run successful with warnings:
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdUtils.sol
  ```
  </details>

#### ✅ <span style="color: green;">cache clean</span>
- **Command**: `forge cache clean`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge cache clean
  ```
  </details>

#### ✅ <span style="color: green;">cache ls</span>
- **Command**: `forge cache ls`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge cache ls
  ```
  </details>

#### ✅ <span style="color: green;">clean</span>
- **Command**: `forge clean`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge cache clean
  ```
  </details>

#### ✅ <span style="color: green;">compiler resolve</span>
- **Command**: `forge compiler resolve --resolc`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge compiler resolve --resolc
  Solidity:
  - Resolc v0.2.0, Solc v0.8.30
  ```
  </details>

#### ✅ <span style="color: green;">config</span>
- **Command**: `forge config`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge config
  ```
  </details>

#### ✅ <span style="color: green;">create</span>
- **Command**: `forge create [OPTIONS] <CONTRACT>`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Required Parameters**: `CONTRACT`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge create Counter --resolc --rpc-url https://testnet-passet-hub-eth-rpc.polkadot.io --private-key 0xacef3f4d5f7c6666e927c24af52f35c45c07990d1f199cd476b0189d1029419f --broadcast --constructor-args 0
  Compiling 1 files with Resolc v0.2.0, Solc v0.8.30
  installing solc version "0.8.30"
  Successfully installed solc 0.8.30
  Resolc 0.2.0, Solc 0.8.30 finished in 2.21s
  Compiler run successful!
  Deployer: 0x2a187c63c5c5212006cBB5D42CCd0BF0F67B142E
  Deployed to: 0xF4ed7573DA31302eCe974692e230Cc9F4D9CE18D
  Transaction hash: 0x4e14189ff8197e8c8c4a0e0ee3c59884b89fd7dfc1a322011506093dc546a88a
  ```
  </details>

#### ✅ <span style="color: green;">doc</span>
- **Command**: `forge doc`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge doc
  ```
  </details>

#### ✅ <span style="color: green;">flatten</span>
- **Command**: `forge flatten [OPTIONS] <PATH>`
- **Required Parameters**: `PATH`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge flatten src/Counter.sol
  // SPDX-License-Identifier: UNLICENSED
  pragma solidity ^0.8.13;

  // src/Counter.sol

  contract Counter {
      uint256 public number;

      function setNumber(uint256 newNumber) public {
          number = newNumber;
      }

      function increment() public {
          number++;
      }
  }
  ```
  </details>

#### ✅ <span style="color: green;">fmt</span>
- **Command**: `forge fmt`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge fmt
  ```
  </details>

#### ✅ <span style="color: green;">geiger</span>
- **Command**: `forge geiger <PATH>`
- **Required Parameters**: `PATH`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge geiger src/Counter.sol
  ```
  </details>

#### ✅ <span style="color: green;">generate test</span>
- **Command**: `forge generate test --contract-name <CONTRACT_NAME>`
- **Required Parameters**: `CONTRACT_NAME`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge generate test --contract-name Counter
  Generated test file: test/Counter.t.sol
  ```
  </details>

#### ✅ <span style="color: green;">generate-fig-spec</span>
- **Command**: `forge generate-fig-spec`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge generate-fig-spec
  ```
  </details>

#### ✅ <span style="color: green;">inspect</span>
- **Command**: `forge inspect`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler. When running with this flag the output for the bytecode should start with `0x505`.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge inspect Counter bytecode --resolc
  ```
  </details>

##### ✅ <span style="color: green;">install</span>
- **Command**: `forge install`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge install vectorized/solady
  Installing solady in /test/lib/solady (url: Some("https://github.com/vectorized/solady"), tag: None)
  Cloning into '/test/lib/solady'...
      Installed solady v0.1.19
  ```
  </details>

#### ✅ <span style="color: green;">update</span>
- **Command**: `forge update`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge update vectorized/solady
  ```
  </details>

#### ✅ <span style="color: green;">remappings</span>
- **Command**: `forge remappings`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge remappings
  forge-std/=lib/forge-std/src/
  solady/=lib/solady/src/
  ```
  </details>

#### ✅ <span style="color: green;">remove</span>
- **Command**: `forge remove`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge remove solady --force
  Removing 'solady' in lib/solady, (url: None, tag: None)
  ```
  </details>

#### ✅ <span style="color: green;">selectors upload</span>
- **Command**: `forge selectors upload`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge selectors upload --all
  Compiling 1 files with Resolc v0.2.0, Solc v0.8.30
  installing solc version "0.8.30"
  Successfully installed solc 0.8.30
  Resolc 0.2.0, Solc 0.8.30 finished in 6.46s
  Compiler run successful with warnings:
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdCheats.sol
  Warning: Warning: Your code or one of its dependencies uses the 'extcodesize' instruction, which is
  usually needed in the following cases:
    1. To detect whether an address belongs to a smart contract.
    2. To detect whether the deploy code execution has finished.
  Polkadot comes with native account abstraction support (so smart contracts are just accounts
  coverned by code), and you should avoid differentiating between contracts and non-contract
  addresses.
  --> lib/forge-std/src/StdUtils.sol
  Uploading selectors for Counter...
  Duplicated: Function increment(): 0xd09de08a
  Duplicated: Function number(): 0x8381f58a
  Duplicated: Function setNumber(uint256): 0x3fb5c1cb
  Selectors successfully uploaded to OpenChain
  ```
  </details>

##### ✅ <span style="color: green;">selectors list</span>
- **Command**: `forge selectors list`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge selectors list
  Listing selectors for contracts in the project...
  Counter

  ╭----------+--------------------+------------╮
  | Type     | Signature          | Selector   |
  +============================================+
  | Function | increment()        | 0xd09de08a |
  |----------+--------------------+------------|
  | Function | number()           | 0x8381f58a |
  |----------+--------------------+------------|
  | Function | setNumber(uint256) | 0x3fb5c1cb |
  ╰----------+--------------------+------------╯
  ```
  </details>

##### ✅ <span style="color: green;">selectors find</span>
- **Command**: `forge selectors find`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge selectors find 0xd09de08a
  Searching for selector "0xd09de08a" in the project...

  Found 1 instance(s)...

  ╭----------+-------------+------------+----------╮
  | Type     | Signature   | Selector   | Contract |
  +================================================+
  | Function | increment() | 0xd09de08a | Counter  |
  ╰----------+-------------+------------+----------╯
  ```
  </details>

#### ✅ <span style="color: green;">selectors cache</span>
- **Command**: `forge selectors cache`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge selectors cache
  Caching selectors for contracts in the project...
  ```
  </details>

#### ✅ <span style="color: green;">tree</span>
- **Command**: `forge tree`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge tree
  ```
  </details>

### Not Working Commands

#### ❌ <span style="color: red;">clone</span>
- **Command**: `forge clone`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge clone
  ```
  </details>

##### ❌ <span style="color: red;">coverage</span>
- **Command**: `forge coverage`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge coverage
  ```
  </details>

#### ❌ <span style="color: red;">snapshot</span>
- **Command**: `forge snapshot`
- **Additional Flags**:
  - `--resolc`: Use the Resolc compiler.
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge snapshot --resolc
  ```
  </details>

#### ❌ <span style="color: red;">test</span>
- **Command**: `forge test`
- **Example**:
  <details>
  <summary>Click to toggle contents of example</summary>

  ```bash
  > forge test
  ```
  </details>
