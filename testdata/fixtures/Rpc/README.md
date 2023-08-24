# Fixture Generation Instructions

### `eth_getLogs.json`

To generate this fixture, send a POST request to a Eth Mainnet (chainId = 1) RPC

```
{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "id": "1",
    "params": [
        {
            "address": "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2",
            "fromBlock": "0x10CEB1B",
            "toBlock": "0x10CEB1B",
            "topics": [
                "0x7fcf532c15f0a6db0bd6d0e038bea71d30d808c7d98cb3bf7268a95bf5081b65"
            ]
        }
    ]
}
```

Then you must change the `address` key to `emitter` because in Solidity, a struct's name cannot be `address` as that is a keyword.
