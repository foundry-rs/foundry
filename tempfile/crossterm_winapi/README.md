![Lines of Code][s7] [![Latest Version][s1]][l1] [![MIT][s2]][l2] [![docs][s3]][l3]

# Crossterm Windows API Abstractions

This crate provides some wrappers aground common used WinAPI functions.
 
The purpose of this library is originally meant for the [crossterm](https://github.com/crossterm-rs/crossterm),
but could be used apart from it. Although, notice that it unstable right because some changes to the
API could be expected.

# Features

This crate provides some abstractions over reading input, console screen buffer, and handle.

The following WinAPI calls:

- CONSOLE_SCREEN_BUFFER_INFO (used to extract information like cursor pos, terminal size etc.)
- CONSOLE_FONT_INFO (used to extract font info like size)
- HANDLE (the handle needed to run functions from WinAPI)
- SetConsoleActiveScreenBuffer (activate an other screen buffer)
- Set/GetConsoleMode (e.g. console modes like disabling output)
- SetConsoleTextAttribute (eg. coloring)
- SetConsoleWindowInfo (changing the buffer location e.g. scrolling)
- FillConsoleOutputAttribute, FillConsoleOutputCharacter (used to replace some block of cells with a color or character.)
- SetConsoleInfo
- ReadConsoleW
- Semaphore object handling

# Example 

The [examples](https://github.com/crossterm-rs/examples) repository has more complete and verbose examples.

## Screen buffer information

```rust 
use crossterm_winapi::{ScreenBuffer, Handle};

fn print_screen_buffer_information() {
    let screen_buffer = ScreenBuffer::current().unwrap();

    // get console screen buffer information
    let csbi = screen_buffer.info().unwrap();

    println!("cursor post: {:?}", csbi.cursor_pos());
    println!("attributes: {:?}", csbi.attributes());
    println!("terminal window dimentions {:?}", csbi.terminal_window());
    println!("terminal size {:?}", csbi.terminal_size());
}
```

## Handle

```rust
use crossterm_winapi::{HandleType, Handle};

fn get_different_handle_types() {
    let out_put_handle = Handle::new(HandleType::OutputHandle).unwrap();
    let out_put_handle = Handle::new(HandleType::InputHandle).unwrap();
    let curr_out_put_handle = Handle::new(HandleType::CurrentOutputHandle).unwrap();
    let curr_out_put_handle = Handle::new(HandleType::CurrentInputHandle).unwrap();
}
```

[s1]: https://img.shields.io/crates/v/crossterm_winapi.svg
[l1]: https://crates.io/crates/crossterm_winapi

[s2]: https://img.shields.io/badge/license-MIT-blue.svg
[l2]: LICENSE

[s3]: https://docs.rs/crossterm_winapi/badge.svg
[l3]: https://docs.rs/crossterm_winapi/

[s7]: https://travis-ci.org/crossterm-rs/crossterm.svg?branch=master
