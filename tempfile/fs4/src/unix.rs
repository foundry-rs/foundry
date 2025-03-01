macro_rules! lock_impl {
    ($file: ty) => {
        #[cfg(not(target_os = "wasi"))]
        pub fn lock_shared(file: &$file) -> std::io::Result<()> {
            flock(file, rustix::fs::FlockOperation::LockShared)
        }

        #[cfg(not(target_os = "wasi"))]
        pub fn lock_exclusive(file: &$file) -> std::io::Result<()> {
            flock(file, rustix::fs::FlockOperation::LockExclusive)
        }

        #[cfg(not(target_os = "wasi"))]
        pub fn try_lock_shared(file: &$file) -> std::io::Result<()> {
            flock(file, rustix::fs::FlockOperation::NonBlockingLockShared)
        }

        #[cfg(not(target_os = "wasi"))]
        pub fn try_lock_exclusive(file: &$file) -> std::io::Result<()> {
            flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        }

        #[cfg(not(target_os = "wasi"))]
        pub fn unlock(file: &$file) -> std::io::Result<()> {
            flock(file, rustix::fs::FlockOperation::Unlock)
        }

        #[cfg(not(target_os = "wasi"))]
        fn flock(file: &$file, flag: rustix::fs::FlockOperation) -> std::io::Result<()> {
            let borrowed_fd = unsafe { rustix::fd::BorrowedFd::borrow_raw(file.as_raw_fd()) };

            match rustix::fs::flock(borrowed_fd, flag) {
                Ok(_) => Ok(()),
                Err(e) => Err(std::io::Error::from_raw_os_error(e.raw_os_error())),
            }
        }
    };
}

#[cfg(any(
    feature = "smol",
    feature = "async-std",
    feature = "tokio",
    feature = "fs-err-tokio"
))]
pub(crate) mod async_impl;
#[cfg(any(feature = "sync", feature = "fs-err"))]
pub(crate) mod sync_impl;

use crate::FsStats;

use std::io::{Error, Result};
use std::path::Path;

pub fn lock_error() -> Error {
    Error::from_raw_os_error(rustix::io::Errno::WOULDBLOCK.raw_os_error())
}

pub fn statvfs(path: impl AsRef<Path>) -> Result<FsStats> {
    match rustix::fs::statvfs(path.as_ref()) {
        Ok(stat) => Ok(FsStats {
            free_space: stat.f_frsize * stat.f_bfree,
            available_space: stat.f_frsize * stat.f_bavail,
            total_space: stat.f_frsize * stat.f_blocks,
            allocation_granularity: stat.f_frsize,
        }),
        Err(e) => Err(std::io::Error::from_raw_os_error(e.raw_os_error())),
    }
}
