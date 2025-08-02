# forge-lsp

A native Language Server Protocol (LSP) implementation for Solidity development using Foundry's compilation and linting infrastructure.

## Usage

Start the LSP server using:

```bash
forge lsp --stdio
```

## Supported LSP Features

### Planned

- [x] forge lint errors
- [ ] Diagnostics (compilation errors and warnings)
- [ ] Go-to-definition
- [ ] Symbol search and references
- [ ] Code completion
- [ ] Hover information
- [ ] Code formatting
- [ ] Refactoring support
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

### Neovim

With `nvim-lspconfig`:

> Install forge nightly with `foundryup -i nightly` to access forge lint feature

```lua
{
  cmd = { "forge", "lsp", "--stdio" },
  filetypes = { "solidity" },
  root_markers = { "foundry.toml", ".git" },
  root_dir = vim.fs.root(0, { "foundry.toml", ".git" }),
}
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
