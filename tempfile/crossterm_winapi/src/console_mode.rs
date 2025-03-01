use std::io::Result;

use winapi::um::consoleapi::{GetConsoleMode, SetConsoleMode};

use super::{result, Handle, HandleType};

/// A wrapper around a screen buffer, focusing on calls to get and set the console mode.
///
/// This wraps [`SetConsoleMode`](https://docs.microsoft.com/en-us/windows/console/setconsolemode)
/// and [`GetConsoleMode`](https://docs.microsoft.com/en-us/windows/console/getconsolemode).
#[derive(Debug, Clone)]
pub struct ConsoleMode {
    // the handle used for the functions of this type.
    handle: Handle,
}

impl ConsoleMode {
    /// Create a new `ConsoleMode` instance.
    ///
    /// This will use the standard output as its handle.
    /// When you explicitly want to specify the handle used for the function calls use `ConsoleMode::from(handle)` instead.
    pub fn new() -> Result<ConsoleMode> {
        Ok(ConsoleMode {
            handle: Handle::new(HandleType::OutputHandle)?,
        })
    }

    /// Set the console mode to the given console mode.
    ///
    /// This function sets the `dwMode`.
    ///
    /// This wraps
    /// [`SetConsoleMode`](https://docs.microsoft.com/en-us/windows/console/setconsolemode).
    pub fn set_mode(&self, console_mode: u32) -> Result<()> {
        result(unsafe { SetConsoleMode(*self.handle, console_mode) })
    }

    /// Get the console mode.
    ///
    /// This function returns the `lpMode`.
    ///
    /// This wraps
    /// [`GetConsoleMode`](https://docs.microsoft.com/en-us/windows/console/getconsolemode).
    pub fn mode(&self) -> Result<u32> {
        let mut console_mode = 0;
        result(unsafe { GetConsoleMode(*self.handle, &mut console_mode) })?;
        Ok(console_mode)
    }
}

impl From<Handle> for ConsoleMode {
    fn from(handle: Handle) -> Self {
        ConsoleMode { handle }
    }
}

#[cfg(test)]
mod tests {
    use super::ConsoleMode;

    // TODO - Test is ignored, because it's failing on Travis CI
    #[test]
    #[ignore]
    fn test_set_get_mode() {
        let mode = ConsoleMode::new().unwrap();

        let original_mode = mode.mode().unwrap();

        mode.set_mode(0x0004).unwrap();
        let console_mode = mode.mode().unwrap();
        assert_eq!(console_mode & 0x0004, mode.mode().unwrap());

        mode.set_mode(original_mode).unwrap();
    }
}
