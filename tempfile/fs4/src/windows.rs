macro_rules! lock_impl {
    ($file: ty) => {
        pub fn lock_shared(file: &$file) -> Result<()> {
            lock_file(file, 0)
        }

        pub fn lock_exclusive(file: &$file) -> Result<()> {
            lock_file(file, LOCKFILE_EXCLUSIVE_LOCK)
        }

        pub fn try_lock_shared(file: &$file) -> Result<()> {
            lock_file(file, LOCKFILE_FAIL_IMMEDIATELY)
        }

        pub fn try_lock_exclusive(file: &$file) -> Result<()> {
            lock_file(file, LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)
        }

        pub fn unlock(file: &$file) -> Result<()> {
            unsafe {
                let ret = UnlockFile(file.as_raw_handle() as HANDLE, 0, 0, !0, !0);
                if ret == 0 {
                    Err(Error::last_os_error())
                } else {
                    Ok(())
                }
            }
        }

        fn lock_file(file: &$file, flags: u32) -> Result<()> {
            unsafe {
                let mut overlapped = mem::zeroed();
                let ret = LockFileEx(
                    file.as_raw_handle() as HANDLE,
                    flags,
                    0,
                    !0,
                    !0,
                    &mut overlapped,
                );
                if ret == 0 {
                    Err(Error::last_os_error())
                } else {
                    Ok(())
                }
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
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetVolumePathNameW,
};

pub fn lock_error() -> Error {
    Error::from_raw_os_error(ERROR_LOCK_VIOLATION as i32)
}

fn volume_path(path: &Path, volume_path: &mut [u16]) -> Result<()> {
    let path_utf8: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let ret = GetVolumePathNameW(
            path_utf8.as_ptr(),
            volume_path.as_mut_ptr(),
            volume_path.len() as u32,
        );
        if ret == 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

pub fn statvfs(path: &Path) -> Result<FsStats> {
    let root_path: &mut [u16] = &mut [0; 261];
    volume_path(path, root_path)?;
    unsafe {
        let mut free_space = 0;
        let mut total_space = 0;
        let ret = GetDiskFreeSpaceExW(
            root_path.as_ptr(),
            &mut free_space,
            &mut total_space,
            std::ptr::null_mut(),
        );
        if ret == 0 {
            return Err(Error::last_os_error());
        }

        let mut sectors_per_cluster = 0;
        let mut bytes_per_sector = 0;
        let mut _number_of_free_clusters = 0;
        let mut _total_number_of_clusters = 0;
        let ret = GetDiskFreeSpaceW(
            root_path.as_ptr(),
            &mut sectors_per_cluster,
            &mut bytes_per_sector,
            &mut _number_of_free_clusters,
            &mut _total_number_of_clusters,
        );
        if ret == 0 {
            Err(Error::last_os_error())
        } else {
            let bytes_per_cluster = sectors_per_cluster as u64 * bytes_per_sector as u64;
            Ok(FsStats {
                free_space,
                available_space: free_space,
                total_space,
                allocation_granularity: bytes_per_cluster,
            })
        }
    }
}
