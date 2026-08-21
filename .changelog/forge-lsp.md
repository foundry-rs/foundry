---
forge: minor
---

Added `forge lsp` to start Solar's Solidity language server without installing the standalone
`solar` executable. The server follows Forge's normal CLI flow, and its default flycheck uses the
running Forge executable while keeping project dotenv prompts away from LSP stdin.
