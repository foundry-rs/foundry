# Foundry Zed extension

Solidity support for [Zed](https://zed.dev), using `forge lsp --stdio` and the
language server's `forge fmt` integration. Install a version of
[Foundry](https://getfoundry.sh) that supports `forge lsp --stdio`.
The extension never searches for, launches, or downloads a standalone Solar.

The extension ID and language server ID remain `solar`; the language remains
`Solidity`. Existing language preferences and initialization options keep their
names. The language server implementation remains in Foundry's `solar_lsp`
dependency.

## Configuration

By default, the extension resolves `forge` from the worktree's shell `PATH` and
checks `forge lsp --stdio --help` before launch. Merely supporting `forge --version` is
insufficient. Missing and incompatible Forge installations produce an actionable
error in Zed's language server status/log. The probe passes `--stdio` explicitly:
Forge accepts it even though it is hidden from help output. The manifest grants
only this probe's argument list permission to execute at a user-selected path.

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
tar -chzf target/editor-dev/zed-solar-local.tar.gz -C editors/zed \
  extension.toml extension.wasm languages grammars/solidity.wasm \
  LICENSE-MIT LICENSE-APACHE GRAMMAR-LICENSE README.md
```

This archive is for local inspection/distribution; store release and registration
are separate maintainer tasks. The `-h` option includes the root license texts
referenced by the extension's license symlinks.

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

## Provenance and license

Migrated from `paradigmxyz/solar`'s `editors/zed` at source revision
`bba703a34e0fabc8587ae7eb794e017e31c5e7ca`. Original authorship is retained in
`extension.toml`; the MIT and Apache-2.0 license files link to the repository
root. The grammar remains
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
