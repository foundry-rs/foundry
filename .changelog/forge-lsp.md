---
forge: minor
forge-lint: patch
---

Added `forge lsp` to start Solar's Solidity language server without installing the standalone
`solar` executable. The server follows Forge's normal CLI flow, and its default flycheck uses the
running Forge executable while keeping protocol output isolated on stdout. Updated the Forge lint
integration for the newer Solar APIs, and ensured compatible clients receive diagnostics through
exactly one delivery method. The embedded server also honors the selected Forge profile when
indexing workspaces and running its default flychecks.
