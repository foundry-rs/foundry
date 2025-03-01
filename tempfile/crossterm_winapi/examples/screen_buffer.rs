#![allow(dead_code)]

#[cfg(windows)]
use std::io::Result;

#[cfg(windows)]
use crossterm_winapi::ScreenBuffer;

#[cfg(windows)]
use crossterm_winapi::FontInfo;

#[cfg(windows)]
fn print_screen_buffer_information() -> Result<()> {
    let screen_buffer = ScreenBuffer::current()?;

    // get console screen buffer information
    let csbi = screen_buffer.info()?;

    println!("cursor post: {:?}", csbi.cursor_pos());
    println!("attributes: {:?}", csbi.attributes());
    println!("terminal window dimentions {:?}", csbi.terminal_window());
    println!("terminal size {:?}", csbi.terminal_size());

    let cfi: FontInfo = screen_buffer.font_info()?;
    println!("font size {:?}", cfi.size());

    Ok(())
}

#[cfg(windows)]
fn multiple_screen_buffers() -> Result<()> {
    // create new screen buffer
    let screen_buffer = ScreenBuffer::create()?;

    // which to this screen buffer
    screen_buffer.show()
}

#[cfg(windows)]
fn main() -> Result<()> {
    print_screen_buffer_information()
}

#[cfg(not(windows))]
fn main() {
    println!("This example is for the Windows platform only.");
}
