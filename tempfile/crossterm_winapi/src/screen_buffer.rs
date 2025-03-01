//! This contains the logic for working with the console buffer.

use std::io::Result;
use std::mem::size_of;

use winapi::{
    shared::minwindef::TRUE,
    shared::ntdef::NULL,
    um::{
        minwinbase::SECURITY_ATTRIBUTES,
        wincon::{
            CreateConsoleScreenBuffer, GetConsoleScreenBufferInfo, GetCurrentConsoleFont,
            SetConsoleActiveScreenBuffer, SetConsoleScreenBufferSize, CONSOLE_TEXTMODE_BUFFER,
            COORD,
        },
        winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE},
    },
};

use super::{handle_result, result, FontInfo, Handle, HandleType, ScreenBufferInfo};

/// A wrapper around a screen buffer.
#[derive(Clone, Debug)]
pub struct ScreenBuffer {
    handle: Handle,
}

impl ScreenBuffer {
    /// Create a wrapper around a screen buffer from its handle.
    pub fn new(handle: Handle) -> Self {
        Self { handle }
    }

    /// Get the current console screen buffer
    pub fn current() -> Result<ScreenBuffer> {
        Ok(ScreenBuffer {
            handle: Handle::new(HandleType::CurrentOutputHandle)?,
        })
    }

    /// Create new console screen buffer.
    ///
    /// This wraps
    /// [`CreateConsoleScreenBuffer`](https://docs.microsoft.com/en-us/windows/console/createconsolescreenbuffer)
    pub fn create() -> Result<ScreenBuffer> {
        let security_attr: SECURITY_ATTRIBUTES = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: NULL,
            bInheritHandle: TRUE,
        };

        let new_screen_buffer = handle_result(unsafe {
            CreateConsoleScreenBuffer(
                GENERIC_READ |           // read/write access
                    GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE, // shared
                &security_attr,                     // default security attributes
                CONSOLE_TEXTMODE_BUFFER,            // must be TEXTMODE
                NULL,
            )
        })?;
        Ok(ScreenBuffer {
            handle: unsafe { Handle::from_raw(new_screen_buffer) },
        })
    }

    /// Set this screen buffer to the current one.
    ///
    /// This wraps
    /// [`SetConsoleActiveScreenBuffer`](https://docs.microsoft.com/en-us/windows/console/setconsoleactivescreenbuffer).
    pub fn show(&self) -> Result<()> {
        result(unsafe { SetConsoleActiveScreenBuffer(*self.handle) })
    }

    /// Get the screen buffer information like terminal size, cursor position, buffer size.
    ///
    /// This wraps
    /// [`GetConsoleScreenBufferInfo`](https://docs.microsoft.com/en-us/windows/console/getconsolescreenbufferinfo).
    pub fn info(&self) -> Result<ScreenBufferInfo> {
        let mut csbi = ScreenBufferInfo::new();
        result(unsafe { GetConsoleScreenBufferInfo(*self.handle, &mut csbi.0) })?;
        Ok(csbi)
    }

    /// Get the current font information like size and font index.
    ///
    /// This wraps
    /// [`GetConsoleFontSize`](https://learn.microsoft.com/en-us/windows/console/getconsolefontsize).
    pub fn font_info(&self) -> Result<FontInfo> {
        let mut fi = FontInfo::new();
        result(unsafe { GetCurrentConsoleFont(*self.handle, 0, &mut fi.0) })?;
        Ok(fi)
    }

    /// Set the console screen buffer size to the given size.
    ///
    /// This wraps
    /// [`SetConsoleScreenBufferSize`](https://docs.microsoft.com/en-us/windows/console/setconsolescreenbuffersize).
    pub fn set_size(&self, x: i16, y: i16) -> Result<()> {
        result(unsafe { SetConsoleScreenBufferSize(*self.handle, COORD { X: x, Y: y }) })
    }

    /// Get the underlying raw `HANDLE` used by this type to execute with.
    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}

impl From<Handle> for ScreenBuffer {
    fn from(handle: Handle) -> Self {
        ScreenBuffer { handle }
    }
}

#[cfg(test)]
mod tests {
    use super::ScreenBuffer;

    #[test]
    fn test_screen_buffer_info() {
        let buffer = ScreenBuffer::current().unwrap();
        let info = buffer.info().unwrap();
        info.terminal_size();
        info.terminal_window();
        info.attributes();
        info.cursor_pos();
    }
}
