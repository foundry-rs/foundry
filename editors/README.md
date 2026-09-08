# Foundry editor integrations

These thin clients use `forge lsp` for Solidity language support. The server is
the `solar_lsp` dependency already embedded in Forge; its formatting handler
delegates to the same Forge executable's `fmt` command. Installing a Foundry
version that supports `forge lsp` is sufficient. No standalone Solar binary is
required or downloaded.

- [VS Code: install, configure, build and package](vscode/README.md).
- [Zed: configure, build and install locally](zed/README.md).

Check `forge lsp --stdio --help`, not just `forge --version`. If the subcommand
is unavailable, upgrade Foundry using [the installation guide](https://getfoundry.sh/introduction/installation)
(`foundryup`, or a newer/nightly build if necessary). To use this checkout:

```sh
cargo build --locked -p forge --bin forge
./target/debug/forge lsp --stdio --help
```

Both extensions resolve Forge from their explicit Forge setting or from the
editor's PATH. That resolved executable also handles formatting and background
Forge checks. Foundry profiles, remappings, workspace discovery, and file
watching remain owned by the existing server. LSP formatting uses the open
document's unsaved contents and the owning project's `foundry.toml`.

Editor builds are independent: Node is only needed for the VS Code client, and
the Zed crate declares its own workspace and lockfile. Ordinary Foundry Cargo
builds do not compile either client. The editor CI checks both clients and
packages VS Code locally; it does not publish to an extension store.

## Extension Development Host

Open the **Foundry repository root** in VS Code, select **Foundry Solidity
Extension** in Run and Debug, and press F5. The root
[`launch.json`](../.vscode/launch.json) loads `editors/vscode`; its
[`tasks.json`](../.vscode/tasks.json) builds this checkout's Forge, installs the
locked Node dependencies, compiles the client, and prepares a disposable profile.
`editors/prepare-vscode.mjs` uses Cargo metadata to find the built Forge, including
custom Cargo target directories, and writes its absolute path to
`solarLsp.forgePath` in that profile.

The separate Extension Development Host opens `target/editor-dev/project`.
Its user data and installed extensions live under `target/editor-dev`, separate
from existing windows and user settings. Open `src/Example.sol`, edit it without
saving, inspect diagnostics/Go to Definition, and run **Format Document**.
The fixture uses two-space indentation. The Extension Host log records the
chosen executable and startup arguments. Do not set the old Solar executable
setting to a Forge path.

## Compatibility and source attribution

The imported snapshot is Solar commit
[`bba703a34e0fabc8587ae7eb794e017e31c5e7ca`](https://github.com/paradigmxyz/solar/commit/bba703a34e0fabc8587ae7eb794e017e31c5e7ca),
from `editors/vscode` and `editors/zed`. The Foundry base at migration is
`ff92f7d3c288b5187fc8c8d8d4e929487ccdf86e`; it locks the Solar crates to
`8d73a1e1fd53980a7f4ae020c821fb92f025616a`. The imported VS Code runtime and Zed
source match the clients at that locked revision; only the VS Code npm
language-client dependency advanced from 10.1.0 to 10.1.1. The locked server
already supports the clients' initialization options, dynamic watchers, indexing,
formatting and command protocol, so this migration does not update Solar.

The source upstream `main` was checked at
`866584c05a475b4face83bd0fa3c56a55abe394a`; its editor files match the imported
snapshot. Checking upstream used a remote ref in Foundry, leaving the source
checkout unchanged. No source checkout is needed to build or run these clients.

Solar authorship and MIT/Apache-2.0 licenses are preserved in each extension.
The VS Code TextMate grammar retains its original license and attribution;
Zed's tree-sitter grammar remains pinned in `extension.toml`.

Public identities are retained: VS Code `solar-lsp`, Solidity language ID,
`solarLsp.*` settings and commands, server-returned `solar.*` commands, and
Zed extension/server ID `solar`. The source VS Code manifest has no publisher;
local packaging preserves that state. Choosing a store publisher and publishing
are separate work. In particular, `solar.copySelector`, `solar.showReferences`
and `solar.showTypeHierarchy` remain registered by the VS Code client.

Executable settings require explicit migration; the old Solar path is never
silently treated as Forge. See each client's README for exact settings and
error handling.

## Remaining work outside Foundry

This change completes the Foundry-side import, not a coordinated cross-repository
release. A follow-up in Solar should remove or redirect `editors/vscode`,
`editors/zed`, `editors/README.md`, and any editor development/installation links
in its documentation. Remove Solar's `editors/vscode` npm Dependabot entry after
ownership moves, and redirect any editor packaging or release references. The
inspected Solar workflows did not contain an editor build or store-publishing
job; recheck at cutover. Keep Solar's language server implementation and its
tests there. Coordinate Zed registry ownership and the VS Code publisher/ID
before publishing; no store publication or source repository deletion is part
of this import.
