//! Implementation for WASI Preview 1
use crate::Error;
use core::mem::MaybeUninit;

pub use crate::util::{inner_u32, inner_u64};

// This linking is vendored from the wasi crate:
// https://docs.rs/wasi/0.11.0+wasi-snapshot-preview1/src/wasi/lib_generated.rs.html#2344-2350
#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn random_get(arg0: i32, arg1: i32) -> i32;
}

pub fn fill_inner(dest: &mut [MaybeUninit<u8>]) -> Result<(), Error> {
    // Based on the wasi code:
    // https://docs.rs/wasi/0.11.0+wasi-snapshot-preview1/src/wasi/lib_generated.rs.html#2046-2062
    // Note that size of an allocated object can not be bigger than isize::MAX bytes.
    // WASI 0.1 supports only 32-bit WASM, so casting length to `i32` is safe.
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let ret = unsafe { random_get(dest.as_mut_ptr() as i32, dest.len() as i32) };
    match ret {
        0 => Ok(()),
        code => {
            let err = u32::try_from(code)
                .map(Error::from_os_error)
                .unwrap_or(Error::UNEXPECTED);
            Err(err)
        }
    }
}
