//! On Linux we try to use the more accurate `utimensat` syscall but this isn't
//! always available so we also fall back to `utimes` if we couldn't find
//! `utimensat` at runtime.

use crate::FileTime;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::prelude::*;
use std::path::Path;
use std::ptr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::SeqCst;

pub fn set_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    set_times(p, Some(atime), Some(mtime), false)
}

pub fn set_file_mtime(p: &Path, mtime: FileTime) -> io::Result<()> {
    set_times(p, None, Some(mtime), false)
}

pub fn set_file_atime(p: &Path, atime: FileTime) -> io::Result<()> {
    set_times(p, Some(atime), None, false)
}

pub fn set_file_handle_times(
    f: &fs::File,
    atime: Option<FileTime>,
    mtime: Option<FileTime>,
) -> io::Result<()> {
    // Attempt to use the `utimensat` syscall, but if it's not supported by the
    // current kernel then fall back to an older syscall.
    static INVALID: AtomicBool = AtomicBool::new(false);
    if !INVALID.load(SeqCst) {
        let times = [super::to_timespec(&atime), super::to_timespec(&mtime)];

        // We normally use a syscall because the `utimensat` function is documented
        // as not accepting a file descriptor in the first argument (even though, on
        // Linux, the syscall itself can accept a file descriptor there).
        #[cfg(not(target_env = "musl"))]
        let rc = unsafe {
            libc::syscall(
                libc::SYS_utimensat,
                f.as_raw_fd(),
                ptr::null::<libc::c_char>(),
                times.as_ptr(),
                0,
            )
        };
        // However, on musl, we call the musl libc function instead. This is because
        // on newer musl versions starting with musl 1.2, `timespec` is always a 64-bit
        // value even on 32-bit targets. As a result, musl internally converts their
        // `timespec` values to the correct ABI before invoking the syscall. Since we
        // use `timespec` from the libc crate, it matches musl's definition and not
        // the Linux kernel's version (for some platforms) so we must use musl's
        // `utimensat` function to properly convert the value. musl's `utimensat`
        // function allows file descriptors in the path argument so this is fine.
        #[cfg(target_env = "musl")]
        let rc = unsafe {
            libc::utimensat(
                f.as_raw_fd(),
                ptr::null::<libc::c_char>(),
                times.as_ptr(),
                0,
            )
        };

        if rc == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOSYS) {
            INVALID.store(true, SeqCst);
        } else {
            return Err(err);
        }
    }

    super::utimes::set_file_handle_times(f, atime, mtime)
}

pub fn set_symlink_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    set_times(p, Some(atime), Some(mtime), true)
}

fn set_times(
    p: &Path,
    atime: Option<FileTime>,
    mtime: Option<FileTime>,
    symlink: bool,
) -> io::Result<()> {
    let flags = if symlink {
        libc::AT_SYMLINK_NOFOLLOW
    } else {
        0
    };

    // Same as the `if` statement above.
    static INVALID: AtomicBool = AtomicBool::new(false);
    if !INVALID.load(SeqCst) {
        let p = CString::new(p.as_os_str().as_bytes())?;
        let times = [super::to_timespec(&atime), super::to_timespec(&mtime)];
        let rc = unsafe { libc::utimensat(libc::AT_FDCWD, p.as_ptr(), times.as_ptr(), flags) };
        if rc == 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOSYS) {
            INVALID.store(true, SeqCst);
        } else {
            return Err(err);
        }
    }

    super::utimes::set_times(p, atime, mtime, symlink)
}
