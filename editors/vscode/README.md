# Foundry Solidity

Solidity support for [VS Code](https://code.visualstudio.com/), powered by
Foundry's `forge lsp`: diagnostics, hover, go to definition, references,
CodeLens and formatting with `forge fmt`. It needs only a recent
[Foundry](https://getfoundry.sh); no standalone Solar installation is used.

## Installation

Install `paradigm.forge-lsp` from the
[Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=paradigm.forge-lsp)
or [Open VSX](https://open-vsx.org/extension/paradigm/forge-lsp).

To try the extension without installing it, run this in your project:

```sh
forge lsp
```

Forge opens the project in VS Code with the extension built in, using a
separate profile that leaves your normal VS Code settings untouched. Run
`forge lsp --help` for more options.

### From source

You need [Node.js](https://nodejs.org) and Git:

```sh
git clone --depth 1 https://github.com/foundry-rs/foundry
cd foundry/editors/vscode
npm ci
npm run package
code --install-extension bundle/forge.vsix
```

## Configuration

The extension runs the `forge` on your `PATH`. To use another Forge, set its
path in your settings:

```json
{
  "solarLsp.forgePath": "/absolute/path/to/forge"
}
```

Relative paths resolve from the first workspace folder. The same Forge serves
the language server, formatting and background checks, and inherits
`FOUNDRY_PROFILE` from the environment.

CodeLens shows selectors, references and inheritance above declarations. Turn
each off with `solarLsp.codeLens.selectors`, `.references` and `.inheritance`,
or all of them with `solarLsp.codeLens.enable`. To trace the language server,
set the `Foundry Solidity` output channel's log level to `Trace`;
`solarLsp.trace.server` selects `messages` or `verbose` detail.

## Formatting

**Format Document** and the `solarLsp.formatDocument` command run `forge fmt` on
the unsaved buffer, using the project's `foundry.toml`.

`solarLsp.formatOnSave` formats on save and defaults to `true`. It yields to
VS Code's `editor.formatOnSave` when that is enabled, so files are formatted
only once. To turn off formatting on save, disable both.

## Migration from Solar

`solarLsp.serverPath` used to select a standalone Solar binary. The extension
now ignores it and warns when it is set. Remove it, and set
`solarLsp.forgePath` if you need a specific Forge. Other `solarLsp.*` settings
and `solar.*` commands keep their names.

## License

Dual licensed under MIT or Apache-2.0. See [NOTICE.md](NOTICE.md) for grammar
attribution.
