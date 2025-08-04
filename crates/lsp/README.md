# Language Server Protocol (`lsp`)

A native Language Server Protocol (LSP) implementation for Solidity development using Foundry's compilation and linting infrastructure.

## Usage

Start the LSP server using:

```bash
forge lsp --stdio
```

## Supported LSP Features

### Planned

- [x] forge lint errors
- [x] Diagnostics (compilation errors and warnings)
- [ ] Go-to-definition
- [ ] Symbol search and references
- [ ] Code completion
- [ ] Hover information
- [ ] Code formatting
- [ ] Code Actions

## Development

### Building

```bash
cargo build --bin forge
```

### Testing

```bash
cargo test -p forge-lsp
```

### VSCode or Cursor

> Install forge nightly with `foundryup -i nightly` to access forge lint feature

You can add the following to VSCode (or cursor) using a lsp-proxy extension see comment [here](https://github.com/foundry-rs/foundry/pull/11187#issuecomment-3148743488):

```json
[
  {
    "languageId": "solidity",
    "command": "forge",
    "fileExtensions": [
      ".sol"
    ],
    "args": [
      "lsp",
      "--stdio"
    ]
  }
]
```

### Neovim

With `nvim-lspconfig`:

> Install forge nightly with `foundryup -i nightly` to access forge lint feature

If you have neovim 0.11+ installed add these to your config

```lua
-- lsp/forge_lsp.lua
{
  cmd = { "forge", "lsp", "--stdio" },
  filetypes = { "solidity" },
  root_markers = { "foundry.toml", ".git" },
  root_dir = vim.fs.root(0, { "foundry.toml", ".git" }),
}
-- init.lua
vim.lsp.enable("forge_lsp")
```

### Debugging in neovim

Lsp logs are stored in `~/.local/state/nvim/lsp.log`

To clear lsp logs run:

```bash
> -f ~/.local/state/nvim/lsp.log
```

To monitor logs in real time run:

```bash
tail -f ~/.local/state/nvim/lsp.log
```

Enable traces in neovim to view full traces in logs:

```sh
:lua vim.lsp.set_log_level("trace")
```

## Contributing

Check out the [foundry contribution guide](https://github.com/foundry-rs/foundry/blob/master/CONTRIBUTING.md).
