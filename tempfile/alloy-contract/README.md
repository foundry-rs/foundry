# alloy-contract

Interact with on-chain contracts.

The main type is `CallBuilder`, which is a builder for constructing calls to on-chain contracts.
It provides a way to encode and decode data for on-chain calls, and to send those calls to the chain.
See its documentation for more details.

## Usage

Combined with the `sol!` macro's `#[sol(rpc)]` attribute, `CallBuilder` can be used to interact with
on-chain contracts. The `#[sol(rpc)]` attribute generates a method for each function in a contract
that returns a `CallBuilder` for that function. See its documentation for more details.

```rust,no_run
# async fn test() -> Result<(), Box<dyn std::error::Error>> {
use alloy_contract::SolCallBuilder;
use alloy_network::Ethereum;
use alloy_primitives::{Address, U256};
use alloy_provider::ProviderBuilder;
use alloy_sol_types::sol;

sol! {
    #[sol(rpc)] // <-- Important! Generates the necessary `MyContract` struct and function methods.
    #[sol(bytecode = "0x1234")] // <-- Generates the `BYTECODE` static and the `deploy` method.
    contract MyContract {
        constructor(address) {} // The `deploy` method will also include any constructor arguments.

        #[derive(Debug)]
        function doStuff(uint a, bool b) public payable returns(address c, bytes32 d);
    }
}

// Build a provider.
let provider = ProviderBuilder::new().on_builtin("http://localhost:8545").await?;

// If `#[sol(bytecode = "0x...")]` is provided, the contract can be deployed with `MyContract::deploy`,
// and a new instance will be created.
let constructor_arg = Address::ZERO;
let contract = MyContract::deploy(&provider, constructor_arg).await?;

// Otherwise, or if already deployed, a new contract instance can be created with `MyContract::new`.
let address = Address::ZERO;
let contract = MyContract::new(address, &provider);

// Build a call to the `doStuff` function and configure it.
let a = U256::from(123);
let b = true;
let call_builder = contract.doStuff(a, b).value(U256::from(50e18 as u64));

// Send the call. Note that this is not broadcasted as a transaction.
let call_return = call_builder.call().await?;
println!("{call_return:?}"); // doStuffReturn { c: 0x..., d: 0x... }

// Use `send` to broadcast the call as a transaction.
let _pending_tx = call_builder.send().await?;
# Ok(())
# }
```
