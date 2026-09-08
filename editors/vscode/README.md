# Solar VS Code extension, maintained in Foundry

Solidity language support for [VS Code](https://code.visualstudio.com/), powered
by Foundry's `forge lsp`. The server reuses `solar_lsp`; formatting delegates to
the same Forge executable's `forge fmt`. No standalone Solar installation is
needed or searched for.

## Installation and configuration

Install or upgrade [Foundry](https://getfoundry.sh) to a build that supports
`forge lsp`. Check `forge lsp --stdio --help`; `forge --version` alone does not
establish LSP support. Source builds use `cargo build --locked -p forge --bin forge`.
The extension resolves `forge` from `PATH`, or uses an explicit executable:

```json
{
  "solarLsp.forgePath": "/absolute/path/to/foundry/target/debug/forge"
}
```

Relative executable paths resolve from the first workspace folder. The resolved
path is shared by LSP, formatting, Foundry configuration loading and automatic
Forge flychecks. `FOUNDRY_PROFILE` is inherited by Forge. Server initialization
keeps the existing `forgePath`, `flychecks`, `codeLens` and `indexing` protocol;
the server discovers project roots, profiles, remappings and bounded file
watchers, including nested projects.

`solarLsp.serverPath` formerly selected standalone Solar. It is deprecated and
ignored, with a migration warning when explicitly configured. Remove it and set
`solarLsp.forgePath` to a **Forge** executable if necessary. Existing Solar paths
are never reinterpreted as Forge paths. The package name `solar-lsp`, language
ID `solidity`, `solarLsp.*` setting keys and `solar.*` commands remain unchanged.
The original manifest has no publisher; local packaging retains that state.
Marketplace ownership and publishing require separate maintainer decisions.

## Formatting

When the server advertises document formatting, both VS Code's formatting
provider and `solarLsp.formatDocument` use that capability. The server sends
unsaved document contents to `forge fmt`, with the document's own Foundry
project configuration. Older compatible servers without formatting get a
client provider using the same resolved Forge and nearest `foundry.toml`.

The legacy `solarLsp.formatOnSave` defaults to `true`. When VS Code's
`editor.formatOnSave` is enabled, the legacy save hook yields to it, avoiding
duplicate formatting. To disable all save formatting, disable both settings.

## Local development

From the **Foundry repository root**:

```bash
cargo build --locked -p forge --bin forge
cd editors/vscode
npm ci
npm run check
npm test
npm run compile
cd ../..
code .
```

Select **Foundry Solidity Extension** and press `F5`. The root launch
configuration loads `editors/vscode` from this checkout and uses the checkout's
`target/debug/forge`. Its preparation task creates an isolated development
project, user-data directory and extensions directory under `target/editor-dev`;
existing windows and user settings are preserved. When opening `editors/vscode`
as the workspace, the included launch/tasks configuration provides the same
setup with paths relative to that directory.

Node dependencies and build output remain local to this directory. Normal
Foundry Cargo builds do not run Node tooling.

### Real Extension Development Host tests

After building Forge, run:

```bash
cd editors/vscode
npm run test:host
```

The test runner uses `../../target/debug/forge` unless `FORGE_PATH` is set.
`VSCODE_EXECUTABLE_PATH` selects an installed VS Code executable; otherwise
`@vscode/test-electron` downloads a test copy. For example on macOS:

```bash
VSCODE_EXECUTABLE_PATH="/Applications/Visual Studio Code.app/Contents/MacOS/Code" npm run test:host
FOUNDRY_EDITOR_TEST_PATH_MODE=1 VSCODE_EXECUTABLE_PATH="/Applications/Visual Studio Code.app/Contents/MacOS/Code" npm run test:host
```

Each run creates a fresh temporary profile, empty extension directory and test
project. It checks the loaded extension path and actual Forge child process,
initialization, syntax diagnostics, symbols, hover, unsaved formatting with a
nested project's two-space format configuration, save formatting and `solar.*`
command registration. `PATH` contains only a link to the selected Forge, so
standalone Solar is unavailable. `FOUNDRY_EDITOR_TEST_PATH_MODE=1` leaves the
Forge setting at its default; `FOUNDRY_EDITOR_TEST_WITH_SOLAR=1` also places a
failing Solar executable on this isolated PATH and asserts it was never run.
Reports are written to `bundle/host-test.json`; `FOUNDRY_EDITOR_TEST_REPORT`
overrides the destination. Test profiles are retained at the printed temporary
path for log inspection. These tests do not edit or close existing user windows.

### Local packaging

```bash
npm run package
npx vsce ls --tree
code --install-extension bundle/solar-lsp.vsix --force
```

Packaging runs the compiler and includes runtime dependencies, grammars,
language configuration and original licenses. The VSIX is local and ignored by
Git. To preserve a normal profile, supply isolated `--user-data-dir` and
`--extensions-dir` arguments when installing or opening VS Code. No automated
Marketplace publishing is configured.

## Protocol tracing and CodeLens

The `Solar LSP` output channel name is retained. Set its log level to `Trace`,
then select `solarLsp.trace.server` `messages` or `verbose` for server execution
tracing. Language-client wire tracing includes request and response payloads.

CodeLens supports selectors, references and inheritance. Select a selector to
copy it, references to open Peek References, or inheritance to open Type
Hierarchy. `solarLsp.codeLens.enable`, `.selectors`, `.references` and
`.inheritance` restart the server when changed. Server-returned `solar.*`
command IDs are registered without renaming them.

## License and maintenance

Dual licensed under MIT or Apache-2.0. See [NOTICE.md](NOTICE.md) for the exact
import revision and grammar attribution. The language-client TypeScript source
is compatible with Foundry's locked `solar_lsp` revision
`8d73a1e1fd53980a7f4ae020c821fb92f025616a`; it supports server formatting,
`forgePath`, CodeLens client commands and dynamic file watchers. No server
implementation is vendored here. VS Code API types are pinned to 1.93.0 to
match the preserved minimum supported editor version.
