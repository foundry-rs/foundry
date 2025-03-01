use std::fmt::Display;
use std::io;

use crate::kb::Key;
use crate::term::Term;

pub use crate::common_term::*;

pub const DEFAULT_WIDTH: u16 = 80;

#[inline]
pub fn is_a_terminal(_out: &Term) -> bool {
    #[cfg(target = "wasm32-wasi")]
    {
        unsafe { libc::isatty(out.as_raw_fd()) != 0 }
    }
    #[cfg(not(target = "wasm32-wasi"))]
    {
        false
    }
}

#[inline]
pub fn is_a_color_terminal(_out: &Term) -> bool {
    // We currently never report color terminals.  For discussion see
    // the issue in the WASI repo: https://github.com/WebAssembly/WASI/issues/162
    false
}

#[inline]
pub fn terminal_size(_out: &Term) -> Option<(u16, u16)> {
    None
}

pub fn read_secure() -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Other,
        "unsupported operation",
    ))
}

pub fn read_single_key(_ctrlc_key: bool) -> io::Result<Key> {
    Err(io::Error::new(
        io::ErrorKind::Other,
        "unsupported operation",
    ))
}

#[inline]
pub fn wants_emoji() -> bool {
    false
}

pub fn set_title<T: Display>(_title: T) {}
