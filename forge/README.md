# `forge`

## Why?

### Write your tests in Solidity to minimize context switching

Writing tests in Javascript/Typescript while writing your smart contracts in
Solidity can be confusing. Forge lets you write your tests in Solidity, so you
can focus on what matters.

```solidity
contract Foo {
    uint256 public x = 1;
    function set(uint256 _x) external {
        x = _x;
    }

    function double() external {
        x = 2 * x;
    }
}

contract FooTest {
    Foo foo;

    // The state of the contract gets reset before each
    // test is run, with the `setUp()` function being called
    // each time after deployment.
    function setUp() public {
        foo = new Foo();
    }

    // A simple unit test
    function testDouble() public {
        require(foo.x() == 1);
        foo.double();
        require(foo.x() == 2);
    }
}
```

### Fuzzing: Go beyond unit testing

When testing smart contracts, fuzzing can uncover edge cases which would be hard
to manually detect with manual unit testing. We support fuzzing natively, where
any test function that takes >0 arguments will be fuzzed, using the
[proptest](https://docs.rs/proptest/1.0.0/proptest/) crate.

An example of how a fuzzed test would look like can be seen below:

```solidity
function testDoubleWithFuzzing(uint256 x) public {
    foo.set(x);
    require(foo.x() == x);
    foo.double();
    require(foo.x() == 2 * x);
}
```

## Features

- [ ] test
  - [x] Simple unit tests
    - [x] Gas costs
    - [x] DappTools style test output
    - [x] JSON test output
    - [x] Matching on regex
    - [x] DSTest-style assertions support
  - [x] Fuzzing
  - [ ] Symbolic execution
  - [ ] Coverage
  - [x] HEVM-style Solidity cheatcodes
    - [x] roll: Sets block.number
    - [x] warp: Sets block.timestamp
    - [x] ffi: Perform foreign function call to terminal
    - [x] store: Sets address storage slot
    - [x] load: Loads address storage slot
    - [x] sign: Signs data
    - [x] addr: Gets address for a private key
    - [x] etch: Sets the contract code at some address
    - [x] deal: Sets account balance
    - [x] prank: Performs a call as another address (changes msg.sender for a
          call)
  - [ ] Structured tracing with abi decoding
  - [ ] Per-line gas profiling
  - [x] Forking mode
  - [x] Automatic solc selection
- [x] build
  - [x] Can read DappTools-style .sol.json artifacts
  - [x] Manual remappings
  - [x] Automatic remappings
  - [x] Multiple compiler versions
  - [x] Incremental compilation
  - [ ] Can read Hardhat-style artifacts
  - [ ] Can read Truffle-style artifacts
- [x] install
- [x] update
- [ ] debug
- [x] CLI Tracing with `RUST_LOG=dapp=trace`

## Future Features

### Dapptools feature parity

Over the next months, we intend to add the following features which are
available in upstream dapptools:

1. Stack Traces
1. Symbolic EVM: The holy grail of testing, symbolically executed EVM allows
1. Invariant Tests
1. Interactive Debugger
1. Code coverage
1. Gas snapshots

### Unique features?

We also intend to add features which are not available in dapptools:

1. Faster tests with parallel EVM execution that produces state diffs instead of
   modifying the state
1. Improved UX for assertions:
   1. Check revert error or reason on a Solidity call
   1. Check that an event was emitted with expected arguments
1. Support more EVM backends (revm, geth's evm, hevm etc.) & benchmark
   performance across them
1. Declarative deployment system based on a config file
1. Formatting & Linting powered by [Solang]()
   1. `dapp fmt`, an automatic code formatter according to standard rules (like
      `prettier-plugin-solidity`)
   1. `dapp lint` a linter + static analyzer. think of this as `solhint` +
      slither + others.
1. Flamegraphs for gas profiling
