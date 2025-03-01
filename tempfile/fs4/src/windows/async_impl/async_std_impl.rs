use std::io::{Error, Result};
use std::mem;
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    FileAllocationInfo, FileStandardInfo, GetFileInformationByHandleEx, LockFileEx,
    SetFileInformationByHandle, UnlockFile, FILE_ALLOCATION_INFO, FILE_STANDARD_INFO,
    LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
};

use async_std::fs::File;

lock_impl!(File);
allocate!(File);
allocate_size!(File);

test_mod! {
  async_std::test,
  use crate::async_std::AsyncFileExt;
  use async_std::fs;
}
