//!Error code utilities
use crate::types::c_int;
use crate::MessageBuf;

use core::{fmt, ptr, slice, cmp};

pub(crate) struct FmtCursor<'a> {
    buf: &'a mut MessageBuf,
    cursor: usize
}

impl<'a> FmtCursor<'a> {
    #[inline(always)]
    fn as_str(&self) -> &'a str {
        unsafe {
            core::str::from_utf8_unchecked(
                slice::from_raw_parts(self.buf.as_ptr() as *const u8, self.cursor)
            )
        }
    }
}

impl<'a> fmt::Write for FmtCursor<'a> {
    #[inline(always)]
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let remaining = self.buf.len().saturating_sub(self.cursor);
        debug_assert!(remaining <= self.buf.len());
        let size = cmp::min(remaining, text.len());
        unsafe {
            ptr::copy_nonoverlapping(text.as_ptr(), self.buf.as_mut_ptr().add(self.cursor) as *mut u8, size);
        }
        self.cursor = self.cursor.saturating_add(size);
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) fn write_message_buf<'a>(out: &'a mut MessageBuf, text: &str) -> &'a str {
    let mut formatter = FmtCursor {
        buf: out,
        cursor: 0,
    };
    let _ = fmt::Write::write_str(&mut formatter, text);
    formatter.as_str()
}

#[inline(always)]
///Maps error code
pub fn generic_map_error_code(code: c_int) -> &'static str {
    match code {
        0 => "Success",
        _ => "Operation failed",
    }
}

pub(crate) fn write_fallback_code(out: &mut MessageBuf, code: c_int) -> &str {
    let mut formatter = FmtCursor {
        buf: out,
        cursor: 0,
    };

    let _ = fmt::Write::write_str(&mut formatter, generic_map_error_code(code));
    formatter.as_str()
}
