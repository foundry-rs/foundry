# forge-lsp

A native Language Server Protocol (LSP) implementation for Solidity development using Foundry's compilation and linting infrastructure.

## Usage

Start the LSP server using:

```bash
forge lsp
```

## Supported LSP Features

### Planned

- [x] Diagnostics (compilation errors and warnings)
- [ ] Go-to-definition
- [ ] Symbol search and references
- [ ] Code completion
- [ ] Hover information
- [ ] Code formatting
- [ ] Refactoring support
- [ ] Code Actions

## Configuration

The LSP server automatically detects Foundry projects by looking for `foundry.toml` files. It uses the same configuration as other Foundry tools.

## Development

### Building

```bash
cargo build --bin forge
```

### Testing

```bash
cargo test -p forge-lsp
```

### Debugging

Use the `--debug` flag to enable debug logging:

```bash
forge lsp
```

### Neovim

With `nvim-lspconfig`:

```lua
{
  cmd = { "forge", "lsp" },
  filetypes = { "solidity" },
  root_markers = { "foundry.toml", ".git" },
  root_dir = vim.fs.root(0, { "foundry.toml", ".git" }),
}
```

## Contributing

Check out the [foundry contribution guide](https://github.com/foundry-rs/foundry/blob/master/CONTRIBUTING.md).
