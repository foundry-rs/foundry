# Foundry Solidity

Solidity support for [Zed](https://zed.dev), powered by Foundry's `forge lsp`:
diagnostics, hover, go to definition, references and formatting with
`forge fmt`. It needs only a recent [Foundry](https://getfoundry.sh); no
standalone Solar installation is used.

## Installation

Open the extensions page with `zed: extensions` and install
**Foundry Solidity** (`forge-lsp`).

### From source

You need Git, a C compiler and [Rust installed with rustup](https://rustup.rs):

```sh
git clone --depth 1 https://github.com/foundry-rs/foundry
```

Then run `zed: install dev extension` from the command palette and select the
`foundry/editors/zed` directory.

## Configuration

The extension runs the `forge` on your worktree's `PATH`, so version managers
can pick each project's Forge. If there is none, it downloads the latest stable
Forge and checks for updates once per session. Install Foundry yourself to keep
the language server and formatter on the same version as your builds.

To use another Forge, set its absolute path in your user or project settings:

```json
{
  "lsp": {
    "solar": {
      "settings": {
        "forgePath": "/absolute/path/to/forge"
      }
    }
  }
}
```

The same Forge serves the language server, formatting and background checks.
It inherits the worktree environment, including `FOUNDRY_PROFILE`; set
`lsp.solar.binary.env` to override variables. If the server fails to start,
check `dev: open language server logs`.

## Formatting

Zed formats Solidity through the language server, which runs `forge fmt` on the
unsaved buffer using the project's `foundry.toml`. Use `editor: format` to
format by hand. To select the formatter explicitly:

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

To turn formatting off, set `languages.Solidity.formatter` to `"none"`.

## Migration from Solar

The language server ID is still `solar`, so existing settings keep working,
with these exceptions:

- Remove `lsp.solar.binary.path` and `lsp.solar.binary.arguments`. The
  extension refuses to start while they are set; use
  `lsp.solar.settings.forgePath` instead.
- Move `lsp.solar.initialization_options.forgePath` to
  `lsp.solar.settings.forgePath`.
- Remove any external `forge fmt` formatter from your settings, or files will
  be formatted twice.

## License

Dual licensed under MIT or Apache-2.0. See [GRAMMAR-LICENSE](GRAMMAR-LICENSE)
for the tree-sitter grammar's license.
