---
forge: minor
---

Running `forge lsp` in a terminal now opens the current Solidity project in VS Code
with a bundled extension, without a Foundry checkout or Node.js installation.
Use `forge lsp --stdio` to run the language server directly, or `--vscode` to open
VS Code when input is redirected.
