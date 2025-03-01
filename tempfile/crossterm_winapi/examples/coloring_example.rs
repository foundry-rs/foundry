#[cfg(windows)]
use std::io::Result;

#[cfg(windows)]
use crossterm_winapi::{Console, ScreenBuffer};

#[cfg(windows)]
fn set_background_color() -> Result<()> {
    // background value
    const BLUE_BACKGROUND: u16 = 0x0010;

    let screen_buffer = ScreenBuffer::current()?;
    let csbi = screen_buffer.info()?;

    // Notice that the color values are stored in wAttribute.
    // So wee need to use bitwise operators to check if the values exists or to get current console colors.
    let attrs = csbi.attributes();
    let fg_color = attrs & 0x0007;

    // apply the blue background flag to the current attributes
    let new_color = fg_color | BLUE_BACKGROUND;

    // set the console text attribute to the new color value.
    Console::from(screen_buffer.handle().clone()).set_text_attribute(new_color)?;

    Ok(())
}

#[cfg(windows)]
fn set_foreground_color() -> Result<()> {
    // background value
    const BLUE_FOREGROUND: u16 = 0x0001;

    let screen_buffer = ScreenBuffer::current()?;
    let csbi = screen_buffer.info()?;

    // Notice that the color values are stored in wAttribute.
    // So we need to use bitwise operators to check if the values exists or to get current console colors.
    let attrs = csbi.attributes();
    let bg_color = attrs & 0x0070;
    let mut color = BLUE_FOREGROUND | bg_color;

    // background intensity is a separate value in attrs,
    // wee need to check if this was applied to the current bg color.
    if (attrs & 0x0080 as u16) != 0 {
        color = color | 0x0080 as u16;
    }

    // set the console text attribute to the new color value.
    Console::from(screen_buffer.handle().clone()).set_text_attribute(color)?;

    Ok(())
}

#[cfg(windows)]
fn main() -> Result<()> {
    set_background_color()?;
    set_foreground_color()
}

#[cfg(not(windows))]
fn main() {
    println!("This example is for the Windows platform only.");
}
