<div align="center">
  <img alt="CliClack Logo" src="https://github.com/fadeevab/cliclack/raw/main/media/cliclack-logo.gif" width="360" />
</div>

<h2 align="center">Effortlessly build beautiful command-line apps with Rust 🦀✨</h2>

[![github.com](https://img.shields.io/badge/cliclack-blue?logo=github&labelColor=black)](https://github.com/fadeevab/cliclack)
[![crates.io](https://img.shields.io/crates/v/cliclack.svg)](https://crates.io/crates/cliclack)
[![docs.rs](https://docs.rs/cliclack/badge.svg)](https://docs.rs/cliclack/)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/fadeevab/cliclack/blob/main/LICENSE)

Beautiful, minimal, opinionated CLI prompts inspired by the
[@clack/prompts](https://www.npmjs.com/package/@clack/prompts) `npm` package.

```sh
cargo add cliclack
```

<h2 align="center"><code>cliclack</code> in action</h2>

```sh
cargo run --example basic
cargo run --example log
```

<div align="center">
  <img alt="CliClack Example" src="https://github.com/fadeevab/cliclack/raw/main/media/cliclack-demo.gif" width="420" />
</div>

💎 Fancy minimal UI<br>
✅ Simple API<br>
🎨 Theme support<br>

### Setup

The `intro` and `outro`/`outro_cancel` functions will
print a message to begin and end a prompt session respectively.

```rust
use cliclack::{intro, outro};

intro("create-my-app")?;
// Do stuff
outro("You're all set!")?;
```

### Input

The input prompt accepts a single line of text trying to parse it into a target type.
Multiline input can be enabled using the `multiline()` switch.

```rust
use cliclack::input;

let path: String = input("Where should we create your project?")
    .placeholder("./sparkling-solid")
    .validate(|input: &String| {
        if input.is_empty() {
            Err("Please enter a path.")
        } else if !input.starts_with("./") {
            Err("Please enter a relative path")
        } else {
            Ok(())
        }
    })
    .interact()?;
```

### 🎨 Theme

A custom theme can be applied for UI rendering. An example of theme customization:

```sh
cargo run --example theme
```

<div align="center">
  <img alt="CliClack Magenta Theme" src="https://github.com/fadeevab/cliclack/raw/main/media/cliclack-theme.png" width="400" />
</div>

### 🚥 Progress Bar

Starting from `cliclack = "0.2.3"`, progress bars and multi-progress bars are supported.

<div align="center">
  <img alt="CliClack Multi Progress Bar" src="https://github.com/fadeevab/cliclack/raw/main/media/cliclack-multi-progress-bar.gif" width="600" />
</div>

### See more

- [Documentation](https://docs.rs/cliclack)
- [Examples](https://github.com/fadeevab/cliclack/tree/main/examples)
