use std::io::{self, Result};
use std::iter;
use std::slice;
use std::str;

use winapi::ctypes::c_void;
use winapi::shared::minwindef::DWORD;
use winapi::shared::ntdef::NULL;
use winapi::um::consoleapi::{GetNumberOfConsoleInputEvents, ReadConsoleInputW, WriteConsoleW};
use winapi::um::wincon::{
    FillConsoleOutputAttribute, FillConsoleOutputCharacterA, GetLargestConsoleWindowSize,
    SetConsoleTextAttribute, SetConsoleWindowInfo, COORD, INPUT_RECORD, SMALL_RECT,
};

use super::{result, Coord, Handle, HandleType, InputRecord, WindowPositions};

/// A wrapper around a screen buffer.
#[derive(Debug, Clone)]
pub struct Console {
    handle: Handle,
}

impl Console {
    /// Create new instance of `Console`.
    ///
    /// This created instance will use the default output handle (STD_OUTPUT_HANDLE) as handle for the function call it wraps.
    pub fn output() -> Result<Console> {
        Ok(Console {
            handle: Handle::new(HandleType::OutputHandle)?,
        })
    }

    /// Sets the attributes of characters written to the console screen buffer by the `WriteFile` or `WriteConsole` functions, or echoed by the `ReadFile` or `ReadConsole` functions.
    /// This function affects text written after the function call.
    ///
    /// The attributes is a bitmask of possible [character
    /// attributes](https://docs.microsoft.com/en-us/windows/console/console-screen-buffers#character-attributes).
    ///
    /// This wraps
    /// [`SetConsoleTextAttribute`](https://docs.microsoft.com/en-us/windows/console/setconsoletextattribute).
    pub fn set_text_attribute(&self, value: u16) -> Result<()> {
        result(unsafe { SetConsoleTextAttribute(*self.handle, value) })?;
        Ok(())
    }

    /// Sets the current size and position of a console screen buffer's window.
    ///
    /// This wraps
    /// [`SetConsoleWindowInfo`](https://docs.microsoft.com/en-us/windows/console/setconsolewindowinfo).
    pub fn set_console_info(&self, absolute: bool, rect: WindowPositions) -> Result<()> {
        let absolute = match absolute {
            true => 1,
            false => 0,
        };
        let a = SMALL_RECT::from(rect);

        result(unsafe { SetConsoleWindowInfo(*self.handle, absolute, &a) })?;

        Ok(())
    }

    /// Writes a character to the console screen buffer a specified number of times, beginning at the specified coordinates.
    /// Returns the number of characters that have been written.
    ///
    /// This wraps
    /// [`FillConsoleOutputCharacterA`](https://docs.microsoft.com/en-us/windows/console/fillconsoleoutputcharacter).
    pub fn fill_whit_character(
        &self,
        start_location: Coord,
        cells_to_write: u32,
        filling_char: char,
    ) -> Result<u32> {
        let mut chars_written = 0;
        result(unsafe {
            // fill the cells in console with blanks
            FillConsoleOutputCharacterA(
                *self.handle,
                filling_char as i8,
                cells_to_write,
                COORD::from(start_location),
                &mut chars_written,
            )
        })?;

        Ok(chars_written)
    }

    /// Sets the character attributes for a specified number of character cells, beginning at the specified coordinates in a screen buffer.
    /// Returns the number of cells that have been modified.
    ///
    /// This wraps
    /// [`FillConsoleOutputAttribute`](https://docs.microsoft.com/en-us/windows/console/fillconsoleoutputattribute).
    pub fn fill_whit_attribute(
        &self,
        start_location: Coord,
        cells_to_write: u32,
        dw_attribute: u16,
    ) -> Result<u32> {
        let mut cells_written = 0;
        // Get the position of the current console window
        result(unsafe {
            FillConsoleOutputAttribute(
                *self.handle,
                dw_attribute,
                cells_to_write,
                COORD::from(start_location),
                &mut cells_written,
            )
        })?;

        Ok(cells_written)
    }

    /// Retrieves the size of the largest possible console window, based on the current text and the size of the display.
    ///
    /// This wraps [`GetLargestConsoleWindowSize`](https://docs.microsoft.com/en-us/windows/console/getlargestconsolewindowsize)
    pub fn largest_window_size(&self) -> Result<Coord> {
        crate::coord_result(unsafe { GetLargestConsoleWindowSize(*self.handle) })
    }

    /// Writes a character string to a console screen buffer beginning at the current cursor location.
    ///
    /// This wraps
    /// [`WriteConsoleW`](https://docs.microsoft.com/en-us/windows/console/writeconsole).
    pub fn write_char_buffer(&self, buf: &[u8]) -> Result<usize> {
        // get string from u8[] and parse it to an c_str
        let utf8 = match str::from_utf8(buf) {
            Ok(string) => string,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Could not parse to utf8 string",
                ));
            }
        };

        let utf16: Vec<u16> = utf8.encode_utf16().collect();
        let utf16_ptr: *const c_void = utf16.as_ptr() as *const _ as *const c_void;

        let mut cells_written: u32 = 0;

        result(unsafe {
            WriteConsoleW(
                *self.handle,
                utf16_ptr,
                utf16.len() as u32,
                &mut cells_written,
                NULL,
            )
        })?;

        Ok(utf8.as_bytes().len())
    }

    /// Read one input event.
    ///
    /// This wraps
    /// [`ReadConsoleInputW`](https://docs.microsoft.com/en-us/windows/console/readconsoleinput).
    pub fn read_single_input_event(&self) -> Result<InputRecord> {
        let mut record: INPUT_RECORD = INPUT_RECORD::default();

        {
            // Convert an INPUT_RECORD to an &mut [INPUT_RECORD] of length 1
            let buf = slice::from_mut(&mut record);
            let num_read = self.read_input(buf)?;

            // The windows API promises that ReadConsoleInput returns at least
            // 1 element
            debug_assert!(num_read == 1);
        }

        Ok(record.into())
    }

    /// Read all available input events without blocking.
    ///
    /// This wraps
    /// [`ReadConsoleInputW`](https://docs.microsoft.com/en-us/windows/console/readconsoleinput).
    pub fn read_console_input(&self) -> Result<Vec<InputRecord>> {
        let buf_len = self.number_of_console_input_events()?;

        // Fast-skipping all the code below if there is nothing to read at all
        if buf_len == 0 {
            return Ok(vec![]);
        }

        let mut buf: Vec<INPUT_RECORD> = iter::repeat_with(INPUT_RECORD::default)
            .take(buf_len as usize)
            .collect();

        let num_read = self.read_input(buf.as_mut_slice())?;

        Ok(buf
            .into_iter()
            .take(num_read)
            .map(InputRecord::from)
            .collect())
    }

    /// Get the number of available input events that can be read without blocking.
    ///
    /// This wraps
    /// [`GetNumberOfConsoleInputEvents`](https://docs.microsoft.com/en-us/windows/console/getnumberofconsoleinputevents).
    pub fn number_of_console_input_events(&self) -> Result<u32> {
        let mut buf_len: DWORD = 0;
        result(unsafe { GetNumberOfConsoleInputEvents(*self.handle, &mut buf_len) })?;
        Ok(buf_len)
    }

    /// Read input (via ReadConsoleInputW) into buf and return the number
    /// of events read. ReadConsoleInputW guarantees that at least one event
    /// is read, even if it means blocking the thread. buf.len() must fit in
    /// a u32.
    fn read_input(&self, buf: &mut [INPUT_RECORD]) -> Result<usize> {
        let mut num_records = 0;
        debug_assert!(buf.len() < std::u32::MAX as usize);

        result(unsafe {
            ReadConsoleInputW(
                *self.handle,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut num_records,
            )
        })?;

        Ok(num_records as usize)
    }
}

impl From<Handle> for Console {
    /// Create a `Console` instance who's functions will be executed on the the given `Handle`
    fn from(handle: Handle) -> Self {
        Console { handle }
    }
}
