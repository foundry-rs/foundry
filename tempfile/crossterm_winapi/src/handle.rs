//! This module contains some logic for working with the console handle.

use std::io::Result;
use std::ops::Deref;
use std::ptr::null_mut;
use std::sync::Arc;

use winapi::shared::minwindef::DWORD;
use winapi::um::{
    fileapi::{CreateFileW, OPEN_EXISTING},
    handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
    processenv::GetStdHandle,
    winbase::{STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE, HANDLE},
};

use super::handle_result;

/// The standard handles of a process.
///
/// See [the Windows documentation on console
/// handles](https://docs.microsoft.com/en-us/windows/console/console-handles) for more info.
#[derive(Debug, Clone, Copy)]
pub enum HandleType {
    /// The process' standard output.
    OutputHandle,
    /// The process' standard input.
    InputHandle,
    /// The process' active console screen buffer, `CONOUT$`.
    CurrentOutputHandle,
    /// The process' console input buffer, `CONIN$`.
    CurrentInputHandle,
}

/// Inner structure for closing a handle on Drop.
///
/// The second parameter indicates if the HANDLE is exclusively owned or not.
/// A non-exclusive handle can be created using for example
/// `Handle::input_handle` or `Handle::output_handle`, which corresponds to
/// stdin and stdout respectively.
#[derive(Debug)]
struct Inner {
    handle: HANDLE,
    is_exclusive: bool,
}

impl Inner {
    fn new_exclusive(handle: HANDLE) -> Self {
        Inner {
            handle,
            is_exclusive: true,
        }
    }

    fn new_shared(handle: HANDLE) -> Self {
        Inner {
            handle,
            is_exclusive: false,
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if self.is_exclusive {
            assert!(
                unsafe { CloseHandle(self.handle) != 0 },
                "failed to close handle"
            )
        }
    }
}

unsafe impl Send for Inner {}

unsafe impl Sync for Inner {}

/// This abstracts away some WinAPI calls to set and get some console handles.
///
/// It wraps WinAPI's [`HANDLE`] type.
#[derive(Debug, Clone)]
pub struct Handle {
    handle: Arc<Inner>,
}

impl Handle {
    /// Create a new handle of a certaint type.
    pub fn new(handle: HandleType) -> Result<Handle> {
        match handle {
            HandleType::OutputHandle => Handle::output_handle(),
            HandleType::InputHandle => Handle::input_handle(),
            HandleType::CurrentOutputHandle => Handle::current_out_handle(),
            HandleType::CurrentInputHandle => Handle::current_in_handle(),
        }
    }

    /// Construct a handle from a raw handle.
    ///
    /// # Safety
    ///
    /// This is unsafe since there is not guarantee that the underlying HANDLE is thread-safe to implement `Send` and `Sync`.
    /// Most HANDLE's however, are thread safe.
    pub unsafe fn from_raw(handle: HANDLE) -> Self {
        Self {
            handle: Arc::new(Inner::new_exclusive(handle)),
        }
    }

    /// Get the handle of the active screen buffer.
    /// When using multiple screen buffers this will always point to the to the current screen output buffer.
    ///
    /// This function uses `CONOUT$` to create a file handle to the current output buffer.
    ///
    /// This wraps
    /// [`CreateFileW`](https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew).
    pub fn current_out_handle() -> Result<Handle> {
        let utf16: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let utf16_ptr: *const u16 = utf16.as_ptr();

        let handle = handle_result(unsafe {
            CreateFileW(
                utf16_ptr,
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        })?;

        Ok(Handle {
            handle: Arc::new(Inner::new_exclusive(handle)),
        })
    }

    /// Get the handle of the console input buffer.
    ///
    /// This function uses `CONIN$` to create a file handle to the current input buffer.
    ///
    /// This wraps
    /// [`CreateFileW`](https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew).
    pub fn current_in_handle() -> Result<Handle> {
        let utf16: Vec<u16> = "CONIN$\0".encode_utf16().collect();
        let utf16_ptr: *const u16 = utf16.as_ptr();

        let handle = handle_result(unsafe {
            CreateFileW(
                utf16_ptr,
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        })?;

        Ok(Handle {
            handle: Arc::new(Inner::new_exclusive(handle)),
        })
    }

    /// Get the handle of the standard output.
    ///
    /// On success this function returns the `HANDLE` to `STD_OUTPUT_HANDLE`.
    ///
    /// This wraps [`GetStdHandle`](https://docs.microsoft.com/en-us/windows/console/getstdhandle)
    /// called with `STD_OUTPUT_HANDLE`.
    pub fn output_handle() -> Result<Handle> {
        Self::std_handle(STD_OUTPUT_HANDLE)
    }

    /// Get the handle of the input screen buffer.
    ///
    /// On success this function returns the `HANDLE` to `STD_INPUT_HANDLE`.
    ///
    /// This wraps [`GetStdHandle`](https://docs.microsoft.com/en-us/windows/console/getstdhandle)
    /// called with `STD_INPUT_HANDLE`.
    pub fn input_handle() -> Result<Handle> {
        Self::std_handle(STD_INPUT_HANDLE)
    }

    fn std_handle(which_std: DWORD) -> Result<Handle> {
        let handle = handle_result(unsafe { GetStdHandle(which_std) })?;

        Ok(Handle {
            handle: Arc::new(Inner::new_shared(handle)),
        })
    }

    /// Checks if the console handle is an invalid handle value.
    ///
    /// This is done by checking if the passed `HANDLE` is equal to `INVALID_HANDLE_VALUE`.
    pub fn is_valid_handle(handle: &HANDLE) -> bool {
        *handle != INVALID_HANDLE_VALUE
    }
}

impl Deref for Handle {
    type Target = HANDLE;

    fn deref(&self) -> &HANDLE {
        &self.handle.handle
    }
}

#[cfg(test)]
mod tests {
    use super::{Handle, HandleType};

    #[test]
    fn test_get_handle() {
        assert!(Handle::new(HandleType::OutputHandle).is_ok());
        assert!(Handle::new(HandleType::InputHandle).is_ok());
        assert!(Handle::new(HandleType::CurrentOutputHandle).is_ok());
        assert!(Handle::new(HandleType::CurrentInputHandle).is_ok());
    }
}
