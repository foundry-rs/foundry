# alloy-json-abi

Full Ethereum [JSON-ABI] implementation.

This crate is a re-implementation of a part of [ethabi]'s API, with a few main
differences:
- the `Contract` struct is now called `JsonAbi` and also contains the `fallback`
  and `receive` functions
- the `Param` and `EventParam` structs only partially parse the type string
  instead of fully resolving it into a Solidity type

[JSON-ABI]: https://docs.soliditylang.org/en/latest/abi-spec.html#json
[ethabi]: https://crates.io/crates/ethabi

## Examples

Parse a JSON ABI file into a `JsonAbi` struct:

```rust
use alloy_json_abi::JsonAbi;

# stringify!(
let path = "path/to/abi.json";
let json = std::fs::read_to_string(path).unwrap();
# );
# let json = "[]";
let abi: JsonAbi = serde_json::from_str(&json).unwrap();
for item in abi.items() {
    println!("{item:?}");
}
```
