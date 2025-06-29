# Browser Wallet

Browser wallet integration for Foundry tools, enabling interaction with MetaMask and other browser-based wallets.

## Overview

This crate provides a bridge between Foundry CLI tools and browser wallets using a local HTTP server. It implements support for:
- Transaction sending via `cast send --browser`
- Contract deployment via `forge create --browser`
- Message signing (personal_sign and eth_signTypedData_v4) via `cast wallet sign --browser`

## Architecture

The browser wallet integration follows this flow:

1. CLI starts a local HTTP server on a random port
2. Opens the user's browser to `http://localhost:PORT`
3. Web interface connects to the browser wallet (e.g., MetaMask)
4. CLI queues requests (transactions/signatures) for browser processing
5. Browser polls for pending requests and prompts user for approval
6. Results are returned to CLI via the HTTP API

## HTTP API Reference

All API endpoints follow JSON-RPC 2.0 conventions and are served from `http://localhost:PORT`.

### GET `/api/heartbeat`
Health check endpoint to verify server is running.

**Response:**
```json
{
  "success": true,
  "data": {
    "status": "ok",
    "connected": true,
    "address": "0x1234..."
  }
}
```

### GET `/api/transaction/pending`
Retrieve the next pending transaction for user approval.

**Response:**
```json
{
  "success": true,
  "data": {
    "id": "uuid-v4",
    "from": "0x1234...",
    "to": "0x5678...",
    "value": "0x1000000000000000",
    "data": "0x...",
    "chainId": "0x1"
  }
}
```

### POST `/api/transaction/response`
Submit transaction approval/rejection result.

**Request:**
```json
{
  "id": "uuid-v4",
  "hash": "0xabcd...",
  "error": null
}
```

**Response:**
```json
{
  "success": true
}
```

### GET `/api/sign/pending`
Retrieve pending message signing request.

**Response:**
```json
{
  "success": true,
  "data": {
    "id": "uuid-v4",
    "message": "Hello World",
    "address": "0x1234...",
    "type": "personal_sign"
  }
}
```

### POST `/api/sign/response`
Submit message signing result.

**Request:**
```json
{
  "id": "uuid-v4",
  "signature": "0xabcd...",
  "error": null
}
```

**Response:**
```json
{
  "success": true
}
```

### GET `/api/network`
Get current network configuration.

**Response:**
```json
{
  "success": true,
  "data": {
    "chainId": 1,
    "name": "mainnet"
  }
}
```

### POST `/api/account`
Update connected wallet account status.

**Request:**
```json
{
  "address": "0x1234...",
  "chainId": 1
}
```

**Response:**
```json
{
  "success": true
}
```

## Message Types

### Transaction Request (`BrowserTransaction`)
```rust
{
  id: String,              // Unique transaction ID
  from: Address,           // Sender address
  to: Option<Address>,     // Recipient (None for contract creation)
  value: U256,             // ETH value to send
  data: Bytes,             // Transaction data
  chainId: ChainId,        // Network chain ID
}
```

### Sign Request (`SignRequest`)
```rust
{
  id: String,              // Unique request ID
  message: String,         // Message to sign
  address: Address,        // Address to sign with
  type: SignType,          // "personal_sign" or "sign_typed_data"
}
```

### Sign Type (`SignType`)
- `personal_sign`: Standard message signing
- `sign_typed_data`: EIP-712 typed data signing

## JavaScript API

The web interface uses the standard EIP-1193 provider interface:

```javascript
// Connect wallet
await window.ethereum.request({ method: 'eth_requestAccounts' });

// Send transaction
const hash = await window.ethereum.request({
  method: 'eth_sendTransaction',
  params: [transaction]
});

// Sign message
const signature = await window.ethereum.request({
  method: 'personal_sign',
  params: [message, address]
});

// Sign typed data
const signature = await window.ethereum.request({
  method: 'eth_signTypedData_v4',
  params: [address, typedData]
});
```

## Security

- Server only accepts connections from localhost
- Content Security Policy headers prevent XSS attacks
- No sensitive data is stored; all operations are transient
- Automatic timeout after 5 minutes of inactivity

## Standards Compliance

- [EIP-1193](https://eips.ethereum.org/EIPS/eip-1193): Ethereum Provider JavaScript API
- [EIP-712](https://eips.ethereum.org/EIPS/eip-712): Typed structured data hashing and signing
- [JSON-RPC 2.0](https://www.jsonrpc.org/specification): Communication protocol

## Contributing

When adding new functionality:
1. Update message types in `lib.rs`
2. Add corresponding HTTP endpoints in `server.rs`
3. Update JavaScript handlers in `assets/web/js/`
4. Add integration tests in `tests/integration/`