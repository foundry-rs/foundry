# Comfy-table

[![GitHub Actions Workflow](https://github.com/Nukesor/comfy-table/actions/workflows/test.yml/badge.svg)](https://github.com/Nukesor/comfy-table/actions/workflows/test.yml)
[![docs](https://docs.rs/comfy-table/badge.svg)](https://docs.rs/comfy-table/)
[![license](http://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/nukesor/comfy-table/blob/main/LICENSE)
[![Crates.io](https://img.shields.io/crates/v/comfy-table.svg)](https://crates.io/crates/comfy-table)
[![codecov](https://codecov.io/gh/nukesor/comfy-table/branch/main/graph/badge.svg)](https://codecov.io/gh/nukesor/comfy-table)

![comfy-table](https://raw.githubusercontent.com/Nukesor/images/main/comfy_table.gif)

<!--- [![dependency status](https://deps.rs/repo/github/nukesor/comfy-table/status.svg)](https://deps.rs/repo/github/nukesor/comfy-table) -->

Comfy-table is designed as a library for building beautiful terminal tables, while being easy to use.

## Table of Contents

- [Features](#features)
- [Examples](#examples)
- [Feature Flags](#feature-flags)
- [Contributing](#contributing)
- [Usage of unsafe](#unsafe)
- [Comparison with other libraries](#comparison-with-other-libraries)

## Features

- Dynamic arrangement of content depending on a given width.
- ANSI content styling for terminals (Colors, Bold, Blinking, etc.).
- Styling Presets and preset modifiers to get you started.
- Pretty much every part of the table is customizable (borders, lines, padding, alignment).
- Constraints on columns that allow some additional control over how to arrange content.
- Cross plattform (Linux, macOS, Windows).
- It's fast enough.
  - Benchmarks show that a pretty big table with complex constraints is build in `470μs` or `~0.5ms`.
  - The table seen at the top of the readme takes `~30μs`.
  - These numbers are from a overclocked `i7-8700K` with a max single-core performance of 4.9GHz.
  - To run the benchmarks yourselves, install criterion via `cargo install cargo-criterion` and run `cargo criterion` afterwards.

Comfy-table is written for the current `stable` Rust version.
Older Rust versions may work but aren't officially supported.

## Examples

```rust
use comfy_table::Table;

fn main() {
    let mut table = Table::new();
    table
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            "This is a text",
            "This is another text",
            "This is the third text",
        ])
        .add_row(vec![
            "This is another text",
            "Now\nadd some\nmulti line stuff",
            "This is awesome",
        ]);

    println!("{table}");
}
```

Create a very basic table.\
This table will become as wide as your content. Nothing fancy happening here.

```text,ignore
+----------------------+----------------------+------------------------+
| Header1              | Header2              | Header3                |
+======================================================================+
| This is a text       | This is another text | This is the third text |
|----------------------+----------------------+------------------------|
| This is another text | Now                  | This is awesome        |
|                      | add some             |                        |
|                      | multi line stuff     |                        |
+----------------------+----------------------+------------------------+
```

### More Features

```rust
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL;
use comfy_table::*;

fn main() {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(40)
        .set_header(vec!["Header1", "Header2", "Header3"])
        .add_row(vec![
            Cell::new("Center aligned").set_alignment(CellAlignment::Center),
            Cell::new("This is another text"),
            Cell::new("This is the third text"),
        ])
        .add_row(vec![
            "This is another text",
            "Now\nadd some\nmulti line stuff",
            "This is awesome",
        ]);

    // Set the default alignment for the third column to right
    let column = table.column_mut(2).expect("Our table has three columns");
    column.set_cell_alignment(CellAlignment::Right);

    println!("{table}");
}
```

Create a table with UTF8 styling, and apply a modifier that gives the table round corners.\
Additionally, the content will dynamically wrap to maintain a given table width.\
If the table width isn't explicitely set and the program runs in a terminal, the terminal size will be used.

On top of this, we set the default alignment for the right column to `Right` and the alignment of the left top cell to `Center`.

```text,ignore
╭────────────┬────────────┬────────────╮
│ Header1    ┆ Header2    ┆    Header3 │
╞════════════╪════════════╪════════════╡
│  This is a ┆ This is    ┆    This is │
│    text    ┆ another    ┆  the third │
│            ┆ text       ┆       text │
├╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌┤
│ This is    ┆ Now        ┆    This is │
│ another    ┆ add some   ┆    awesome │
│ text       ┆ multi line ┆            │
│            ┆ stuff      ┆            │
╰────────────┴────────────┴────────────╯
```

### Styling

```rust
use comfy_table::presets::UTF8_FULL;
use comfy_table::*;

fn main() {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(80)
        .set_header(vec![
            Cell::new("Header1").add_attribute(Attribute::Bold),
            Cell::new("Header2").fg(Color::Green),
            Cell::new("Header3"),
        ])
        .add_row(vec![
             Cell::new("This is a bold text").add_attribute(Attribute::Bold),
             Cell::new("This is a green text").fg(Color::Green),
             Cell::new("This one has black background").bg(Color::Black),
        ])
        .add_row(vec![
            Cell::new("Blinky boi").add_attribute(Attribute::SlowBlink),
            Cell::new("This table's content is dynamically arranged. The table is exactly 80 characters wide.\nHere comes a reallylongwordthatshoulddynamicallywrap"),
            Cell::new("COMBINE ALL THE THINGS")
                .fg(Color::Green)
                .bg(Color::Black)
                .add_attributes(vec![
                    Attribute::Bold,
                    Attribute::SlowBlink,
                ])
        ]);

    println!("{table}");
}
```

This code generates the table that can be seen at the top of this document.

### Code Examples

A few examples can be found in the `example` folder.
To test an example, run `cargo run --example $name`. E.g.:

```bash
cargo run --example readme_table
```

If you're looking for more information, take a look at the [tests folder](https://github.com/Nukesor/comfy-table/tree/main/tests).
There are tests for almost every feature including a visual view for each resulting table.

## Feature Flags

### `tty` (enabled)

This flag enables support for terminals. In detail this means:

- Automatic detection whether we're in a terminal environment.
  Only used when no explicit `Table::set_width` is provided.
- Support for ANSI Escape Code styling for terminals.

### `custom_styling` (disabled)

This flag enables support for custom styling of text inside of cells.

- Text formatting still works, even if you roll your own ANSI escape sequences.
- Rainbow text
- Makes comfy-table 30-50% slower

### `reexport_crossterm` (disabled)

With this flag, comfy-table re-exposes crossterm's [`Attribute`](https://docs.rs/crossterm/latest/crossterm/style/enum.Attribute.html) and [`Color`](https://docs.rs/crossterm/latest/crossterm/style/enum.Color.html) enum.
By default, a mirrored type is exposed, which internally maps to the crossterm type.

This feature is very convenient if you use both comfy-table and crossterm in your code and want to use crossterm's types for everything interchangeably.

**BUT** if you enable this feature, you opt-in for breaking changes on minor/patch versions.
Meaning, you have to update crossterm whenever you update comfy-table and you **cannot** update crossterm until comfy-table released a new version with that crossterm version.

## Contributing

Comfy-table's main focus is on being minimalistic and reliable.
A fixed set of features that just work for "normal" use-cases:

- Normal tables (columns, rows, one cell per column/row).
- Dynamic arrangement of content to a given width.
- Some kind of manual intervention in the arrangement process.

If you come up with an idea or an improvement that fits into the current scope of the project, feel free to create an issue :)!

Some things however will most likely not be added to the project since they drastically increase the complexity of the library or cover very specific edge-cases.

Such features are:

- Nested tables
- Cells that span over multiple columns/rows
- CSV to table conversion and vice versa

## Unsafe

Comfy-table doesn't allow `unsafe` code in its code-base.
As it's a "simple" formatting library it also shouldn't be needed in the future.

If one disables the `tty` feature flag, this is also true for all of its dependencies.

However, when enabling `tty`, Comfy-table uses one unsafe function call in its dependencies. \
It can be circumvented by explicitely calling [Table::force_no_tty](https://docs.rs/comfy-table/latest/comfy_table/struct.Table.html#method.force_no_tty).

1. `crossterm::terminal::size`. This function is necessary to detect the current terminal width if we're on a tty.
   This is only called if no explicit width is provided via `Table::set_width`.

   <http://rosettacode.org/wiki/Terminal_control/Dimensions#Library:_BSD_libc>
   This is another libc call which is used to communicate with `/dev/tty` via a file descriptor.

   ```rust,ignore
   ...
   if wrap_with_result(unsafe { ioctl(fd, TIOCGWINSZ.into(), &mut size) }).is_ok() {
       Ok((size.ws_col, size.ws_row))
   } else {
       tput_size().ok_or_else(|| std::io::Error::last_os_error().into())
   }
   ...
   ```

## Comparison with other libraries

The following are official statements of the other crate authors.
[This ticket](https://github.com/Nukesor/comfy-table/issues/76) can be used as an entry to find all other sibling tickets in the other projects.

### Cli-table

The main focus of [`cli-table`](https://crates.io/crates/cli-table) is to support all platforms and at the same time limit the dependencies to keep the compile times and crate size low.

Currently, this crate only pulls two external dependencies (other than cli-table-derive):

- termcolor
- unicode-width

With csv feature enabled, it also pulls csv crate as dependency.

### Term-table

[`term-table`](https://crates.io/crates/term-table) is pretty basic in terms of features.
My goal with the project is to provide a good set of tools for rendering CLI tables, while also allowing users to bring their own tools for things like colours.
One thing that is unique to `term-table` (as far as I'm aware) is the ability to have different number of columns in each row of the table.

### Prettytables-rs

[`prettytables-rs`](https://crates.io/crates/prettytable-rs) provides functionality for formatting and aligning tables.
It his however abandoned since over three years and a [rustsec/advisory-db](https://github.com/rustsec/advisory-db/issues/1173) entry has been requested.

### Comfy-table

One of [`comfy-table`](https://crates.io/crates/comfy-table)'s big foci is on providing a minimalistic, but rock-solid library for building text-based tables.
This means that the code is very well tested, no usage of `unsafe` and `unwrap` is only used if we can be absolutely sure that it's safe.
There're only one occurrence of `unsafe` in all of comfy-table's dependencies, to be exact inside the `tty` communication code, which can be explicitly disabled.

The other focus is on dynamic-length content arrangement.
This means that a lot of work went into building an algorithm that finds a (near) optimal table layout for any given text and terminal width.
