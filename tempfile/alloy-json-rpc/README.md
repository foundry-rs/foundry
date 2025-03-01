# alloy-json-rpc

Core types for JSON-RPC 2.0 clients.

This crate includes data types and traits for JSON-RPC 2.0 requests and
responses, targeted at RPC client usage.

### Core Model

<!-- TODO: More links and real doctests -->

A JSON-RPC 2.0 request is a JSON object containing an ID, a method name, and
an arbitrary parameters object. The parameters object may be omitted if empty.

Any object that may be Serialized and Cloned may be used as RPC Parameters.

Requests are sent via transports (see [alloy-transports]). This results in 1 of
3 outcomes, captured in the `RpcResult<E>` enum:

- `Ok(Response)` - The request was successful, and the server returned a
  response.
- `ErrResp(ErrorPayload)` - The request was received by the server. Server-side
  handling was unsuccessful, and the server returned an error response. This
  indicates a server-side error.
- `Err(E)` - Some client-side error prevented the request from being received
  by the server, or prevented the response from being processed. This indicates a client-side or transport-related error.

[alloy-transports]: ../transports

### Limitations

- This library does not support borrowing response data from the deserializer.
  This is intended to simplify client implementations, but makes the library
  poorly suited for use in a high-performance server.
