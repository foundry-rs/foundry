# Browser wallet regression tests

These tests run a real Cast binary and its embedded browser app against local Anvil
and a rejecting EIP-1193 mock provider. They do not sign or broadcast transactions.
They require Node.js and Playwright with Chromium installed, and run separately
from the Rust CLI suite. Port 9545 must be free because the embedded app currently
uses that fixed port for API requests.

From the repository root:

```sh
cargo build --bin cast --bin anvil
browser_deps=$(mktemp -d)
npm install --prefix "$browser_deps" playwright@1.57.0
"$browser_deps/node_modules/.bin/playwright" install chromium
NODE_PATH="$browser_deps/node_modules" node --test crates/cast/tests/cli/browser/legacy.test.cjs
```

Set `CAST_BIN` and `ANVIL_BIN` to absolute paths to test other binary locations.
The test checks the complete request received by the wallet and verifies that
Cast reports the rejection without printing a transaction hash.
