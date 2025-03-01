#![cfg(windows)]
#![deny(unused_imports)]

use std::io;

use winapi::shared::minwindef::BOOL;
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::wincontypes::COORD;
use winapi::um::winnt::HANDLE;

pub use self::{
    cfi::FontInfo,
    console::Console,
    console_mode::ConsoleMode,
    csbi::ScreenBufferInfo,
    handle::{Handle, HandleType},
    screen_buffer::ScreenBuffer,
    semaphore::Semaphore,
    structs::{
        ButtonState, ControlKeyState, Coord, EventFlags, InputRecord, KeyEventRecord, MouseEvent,
        Size, WindowPositions,
    },
};

mod cfi;
mod console;
mod console_mode;
mod csbi;
mod handle;
mod screen_buffer;
mod semaphore;
mod structs;

/// Get the result of a call to WinAPI as an [`io::Result`].
#[inline]
pub fn result(return_value: BOOL) -> io::Result<()> {
    if return_value != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Get the result of a call to WinAPI that returns a
/// [`COORD`](https://docs.microsoft.com/en-us/windows/console/coord-str) as an [`io::Result`].
#[inline]
pub fn coord_result(return_value: COORD) -> io::Result<Coord> {
    if return_value.X != 0 && return_value.Y != 0 {
        Ok(Coord::from(return_value))
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Get the result of a call to WinAPI that returns a handle or `INVALID_HANDLE_VALUE`.
#[inline]
pub fn handle_result(return_value: HANDLE) -> io::Result<HANDLE> {
    if return_value != INVALID_HANDLE_VALUE {
        Ok(return_value)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Get the result of a call to WinAPI that returns a handle or `NULL`.
#[inline]
pub fn nonnull_handle_result(return_value: HANDLE) -> io::Result<HANDLE> {
    if return_value.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(return_value)
    }
}
