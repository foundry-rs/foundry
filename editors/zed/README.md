# Foundry Solidity

Solidity support for [Zed](https://zed.dev), using `forge lsp --stdio` and the
language server's `forge fmt` integration. It uses the `forge` on your `PATH`,
which must come from a [Foundry](https://getfoundry.sh) release that supports
`forge lsp --stdio`, and downloads the latest stable Forge when there is none.
The extension never searches for or launches a standalone Solar.

The extension ID is `forge-lsp`, matching the VS Code extension. The language
server ID remains `solar` and the language remains `Solidity`. Existing language
preferences and initialization options keep their names. The language server
implementation remains in Foundry's `solar_lsp` dependency.

## Configuration

By default, the extension resolves `forge` from the worktree's shell `PATH`.
Zed launches `forge lsp --stdio` in the worktree directory, allowing version-manager
shims to select the project's Forge installation. The extension does not run a
separate capability probe because Zed's process API cannot set its working
directory. Missing custom executables and incompatible builds are reported by
Zed's language server status/log during startup. To check an installation, run
`forge lsp --stdio --help` from the worktree directory.

If `forge` is not on `PATH` and `forgePath` is unset, the extension downloads
`forge` from the latest stable Foundry GitHub release into its own directory. It
checks for a newer release once per Zed session, keeps only `forge` from the
archive, removes older downloads, and reuses the last complete download when the
check or download fails. Prebuilt Forge exists for macOS and Linux on x86-64 and
ARM64, and for Windows on x86-64. On Linux the extension uses the static musl
build, which does not depend on the system glibc. On other platforms, or when
the download fails with nothing to fall back on, the extension shows
installation guidance. Installing Foundry yourself keeps the language server and
formatter on the same Forge version as your builds.

To select a specific Forge installation, set its absolute path in your Zed user
or project settings:

```json
{
  "lsp": {
    "solar": {
      "settings": {
        "forgePath": "/absolute/path/to/foundry/target/debug/forge"
      }
    }
  }
}
```

Windows paths are supported as well, for example `C:\\Foundry\\forge.exe`.
This one setting selects the language server, formatter, and Forge checks. The
extension sends the selected path as the existing `forgePath` initialization
option. The worktree environment is inherited, including `FOUNDRY_PROFILE`;
`lsp.solar.binary.env` can supply environment overrides.

### Migration from Solar

Remove old `lsp.solar.binary.path` and `lsp.solar.binary.arguments` overrides.
Zed applies those directly and bypasses the extension's command resolution, so
they retain their original raw language server meaning; they are not reinterpreted
as Forge paths. The extension rejects those overrides during initialization with
migration instructions. Use `lsp.solar.settings.forgePath` for Forge instead.

If you set `lsp.solar.initialization_options.forgePath`, move that value to
`lsp.solar.settings.forgePath` and remove the old entry. A conflicting old value
is rejected instead of silently mixing Forge installations. Other initialization
options pass through unchanged.

Remove any external `forge fmt` formatter override left from the old extension.
Zed's default language server formatting uses the current unsaved buffer and the
owning Foundry project's formatter configuration. To select it explicitly:

```json
{
  "languages": {
    "Solidity": {
      "language_servers": ["solar", "!solidity"],
      "formatter": { "language_server": { "name": "solar" } },
      "format_on_save": "on"
    }
  }
}
```

Manual formatting is available through Zed's `editor: format` command. Do not
combine external formatting and language server formatting in a formatter list:
that would format the document twice. To disable formatting, set
`languages.Solidity.formatter` to `"none"`.

## Build and local installation

Run these from the Foundry repository root. The extension has its own Cargo
workspace and lockfile, so normal Foundry builds do not build Zed or WASM.

```sh
cargo build --locked -p forge --bin forge
./target/debug/forge lsp --stdio --help
cargo test --manifest-path editors/zed/Cargo.toml --locked
cargo clippy --manifest-path editors/zed/Cargo.toml --locked --all-targets -- -D warnings
cargo +nightly fmt --manifest-path editors/zed/Cargo.toml -- --check
rustup target add wasm32-wasip2
cargo build --manifest-path editors/zed/Cargo.toml --locked --release --target wasm32-wasip2
```

The standalone build produces
`editors/zed/target/wasm32-wasip2/release/zed_solar.wasm`. Zed's dev extension
installer builds that component, fetches the pinned tree-sitter grammar, and
places `extension.wasm` and `grammars/solidity.wasm` in this extension directory.
These generated files are ignored by Git. Rust and a C compiler are required for
the dev installer, with network access for uncached dependencies and the grammar.

After dev installation has built both WASM files, a local portable archive can
be created without copying the Cargo target directory or grammar checkout:

```sh
mkdir -p target/editor-dev
tar -czf target/editor-dev/zed-solar-local.tar.gz -C editors/zed \
  extension.toml extension.wasm languages grammars/solidity.wasm \
  GRAMMAR-LICENSE README.md -C ../.. LICENSE-MIT LICENSE-APACHE
```

This archive is for local inspection/distribution; see [Publishing](#publishing)
for registry releases. The archive reads the license texts directly from
the repository root, including in checkouts with `core.symlinks=false`.

1. Launch Zed with a new `--user-data-dir`; its settings live in `config/` under
   that directory. On macOS, the normal CLI can forward to an existing instance
   even with this flag. To keep existing windows and settings separate, launch
   the app executable directly from a terminal with its stateless test mode:

   ```sh
   ZED_STATELESS=1 /Applications/Zed.app/Contents/MacOS/zed \
     --user-data-dir "$PWD/target/editor-dev/zed-user-data" \
     "$PWD/target/editor-dev/project"
   ```

   Run `node editors/prepare-vscode.mjs` first to create that scratch project.
   Stateless mode uses an in-memory session database. Terminal output contains
   the startup logs; retain it for process verification. On other platforms use
   a separately isolated instance with `zed --user-data-dir <new-directory>`.
2. Configure `lsp.solar.settings.forgePath` in that isolated profile to the
   absolute path of **this checkout's** `target/debug/forge`.
3. Run `zed: install dev extension` from the command palette and select this
   repository's `editors/zed` directory.
4. Open a Foundry project and a Solidity file. In `dev: open language server
   logs`, check that the server command is the selected `forge lsp --stdio`.
   Check diagnostics, hover/navigation, and format an unsaved edit while using
   a distinctive `[fmt]` setting in the project's `foundry.toml`.
   Trust only the scratch worktree when Zed prompts, so its language server can
   start.
5. After extension changes, run `zed: rebuild dev extension` to rebuild the local
   dev extension. Only close the isolated test window when finished.

The bundled tree-sitter queries and pinned grammar reference are required
resources; keep them alongside `extension.toml`. No source repository checkout
or separately installed `solar` is needed.

## Publishing

Zed builds and hosts extensions listed in the
[`zed-industries/extensions`](https://github.com/zed-industries/extensions)
registry. To release, bump `version` in `extension.toml` and `Cargo.toml` and
merge that to `master`. Then open a registry PR that points the
`extensions/forge-lsp` submodule at the merged commit and sets the same version
in `extensions.toml`:

```toml
[forge-lsp]
submodule = "extensions/forge-lsp"
path = "editors/zed"
version = "0.1.0"
```

The submodule URL must be `https://github.com/foundry-rs/foundry.git`. The first
release adds the submodule with `git submodule add`; later releases only move
it. Run `pnpm sort-extensions` before opening the PR. Registry CI builds the
extension from source with the `zed-extension` CLI and checks that the extension
IDs match and that `LICENSE-MIT` or `LICENSE-APACHE` holds a known license text.

## Provenance and license

Migrated from `paradigmxyz/solar`'s `editors/zed` at source revision
`bba703a34e0fabc8587ae7eb794e017e31c5e7ca`. The MIT and Apache-2.0 license
files link to the repository root. The grammar remains
pinned to `JoranHonig/tree-sitter-solidity` commit
`048fe686cb1fde267243739b8bdbec8fc3a55272`. Its MIT copyright notice is retained
in [GRAMMAR-LICENSE](GRAMMAR-LICENSE), including in local archives.

Foundry currently locks `solar_lsp` at
`8d73a1e1fd53980a7f4ae020c821fb92f025616a`. That revision already accepts
`forgePath`, advertises document formatting, and delegates unsaved source to
`forge fmt --raw --root <project> -`; this migration requires no dependency
update. Future changes must check the revision in Foundry's `Cargo.lock` rather
than assume that all newer Solar functionality is available.

Dual licensed under MIT or Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).
