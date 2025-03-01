# FAQ

## Why is my styling broken? Why doesn't styling work?

`comfy-table` only supports styling via the internal styling functions on [Cell](https://docs.rs/comfy-table/5.0.0/comfy_table/struct.Cell.html#method.fg).

Any styling from other libraries, even crossterm, will most likely not work as expected or break.

It's impossible for `comfy-table` to know about any ANSI escape sequences it doesn't create itself.
Hence, it's not possible to respect unknown styling, as ANSI styling doesn't work this way and doesn't support this.

If you come up with a solution to this problem, feel free to create a PR.
