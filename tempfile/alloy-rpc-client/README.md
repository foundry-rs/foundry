# alloy-rpc-client

Low-level Ethereum JSON-RPC client implementation.

## Usage

Usage of this crate typically means instantiating an `RpcClient` over some
`Transport`. The RPC client can then be used to make requests to the RPC
server. Requests are captured as `RpcCall` futures, which can then be polled to
completion.

For example, to make a simple request:

```rust,ignore
// Instantiate a new client over a transport.
let client: ReqwestClient = ClientBuilder::default().http(url);

// Prepare a request to the server.
let request = client.request_noparams("eth_blockNumber");

// Poll the request to completion.
let block_number = request.await.unwrap();
```

Batch requests are also supported:

```rust,ignore
// Instantiate a new client over a transport.
let client: ReqwestClient = ClientBuilder::default().http(url);

// Prepare a batch request to the server.
let batch = client.new_batch();

// Batches serialize params immediately. So we need to handle the result when
// adding calls.
let block_number_fut = batch.add_call("eth_blockNumber", ()).unwrap();
let balance_fut = batch.add_call("eth_getBalance", address).unwrap();

// Make sure to send the batch!
batch.send().await.unwrap();

// After the batch is complete, we can get the results.
// Note that requests may error separately!
let block_number = block_number_fut.await.unwrap();
let balance = balance_fut.await.unwrap();
```
