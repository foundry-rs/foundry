# Foundry editor integrations

Install [VS Code](https://code.visualstudio.com/) and a recent
[Foundry](https://getfoundry.sh/introduction/installation), then run this from
your Solidity project in a terminal:

```sh
forge lsp
```

Forge opens the current project in a VS Code Extension Development Host with
the Solidity extension included in the Forge binary. Open a `.sol` file to use
diagnostics, Go to Definition, hover and formatting. There is no Foundry checkout
or Node/npm requirement. The server is the `solar_lsp` dependency embedded in
Forge; formatting uses the same Forge executable's `fmt` command.

To select a project or VS Code installation explicitly:

```sh
forge lsp /path/to/project
forge lsp --code-path /path/to/code
```

The launcher finds `code` on PATH. On macOS, it also checks the VS Code app in
`/Applications`. If standard input is redirected, use `forge lsp --vscode` to
open VS Code. A project path or `--code-path` also selects the launcher.
`forge lsp --stdio` always runs the language server for an editor client; bare
`forge lsp` with redirected input preserves that behavior.

The launcher uses dedicated persistent VS Code profiles under
`~/.foundry/cache/lsp/vscode`, leaving normal VS Code settings untouched.
Each project, Forge executable path and selected Foundry profile gets its own
profile. Bundled extension assets are cached by content under
`~/.foundry/cache/lsp/extensions`. No extension store installation or standalone
Solar binary is required.

- [VS Code: install, configure, build and package](vscode/README.md).
- [Zed: configure, build and install locally](zed/README.md).

Check `forge lsp --help` for `--vscode` to confirm launcher support. If it is
unavailable, upgrade Foundry (`foundryup`, or a newer/nightly build if necessary).
To use this checkout:

```sh
cargo build --locked -p forge --bin forge
./target/debug/forge lsp /path/to/project
```

The launched VS Code client uses the Forge executable that opened it, including
for formatting and background checks. Separately installed VS Code and Zed
extensions resolve Forge from their explicit Forge setting or the editor's PATH.
Foundry profiles, remappings, workspace discovery, and file watching remain
owned by the existing server. LSP formatting uses the open document's unsaved
contents and the owning project's `foundry.toml`.

Node is needed only to develop or rebuild the VS Code client. Its committed
runtime bundle is embedded during ordinary Cargo builds without Node tooling.
The Zed crate declares its own workspace and lockfile. Editor CI checks both
clients, verifies the embedded bundle matches its source and packages VS Code
locally; it does not publish to an extension store.

## Debugging extension changes

To develop the client itself, open the **Foundry repository root** in VS Code,
select **Foundry Solidity Extension** in Run and Debug, and press F5. The root
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

Solar authorship is preserved in each extension. Its MIT/Apache-2.0 license
files link to the repository root licenses.
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
