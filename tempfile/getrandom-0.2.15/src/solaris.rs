//! Solaris implementation using getrandom(2).
//!
//! While getrandom(2) has been available since Solaris 11.3, it has a few
//! quirks not present on other OSes. First, on Solaris 11.3, calls will always
//! fail if bufsz > 1024. Second, it will always either fail or completely fill
//! the buffer (returning bufsz). Third, error is indicated by returning 0,
//! rather than by returning -1. Finally, "if GRND_RANDOM is not specified
//! then getrandom(2) is always a non blocking call". This _might_ imply that
//! in early-boot scenarios with low entropy, getrandom(2) will not properly
//! block. To be safe, we set GRND_RANDOM, mirroring the man page examples.
//!
//! For more information, see the man page linked in lib.rs and this blog post:
//! https://blogs.oracle.com/solaris/post/solaris-new-system-calls-getentropy2-and-getrandom2
//! which also explains why this crate should not use getentropy(2).
use crate::{util_libc::last_os_error, Error};
use core::mem::MaybeUninit;

const MAX_BYTES: usize = 1024;

pub fn getrandom_inner(dest: &mut [MaybeUninit<u8>]) -> Result<(), Error> {
    for chunk in dest.chunks_mut(MAX_BYTES) {
        let ptr = chunk.as_mut_ptr() as *mut libc::c_void;
        let ret = unsafe { libc::getrandom(ptr, chunk.len(), libc::GRND_RANDOM) };
        // In case the man page has a typo, we also check for negative ret.
        if ret <= 0 {
            return Err(last_os_error());
        }
        // If getrandom(2) succeeds, it should have completely filled chunk.
        if (ret as usize) != chunk.len() {
            return Err(Error::UNEXPECTED);
        }
    }
    Ok(())
}
