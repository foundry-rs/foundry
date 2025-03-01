macro_rules! allocate_size {
    ($file: ty) => {
        pub async fn allocated_size(file: &$file) -> Result<u64> {
            unsafe {
                let mut info: FILE_STANDARD_INFO = mem::zeroed();

                let ret = GetFileInformationByHandleEx(
                    file.as_raw_handle() as HANDLE,
                    FileStandardInfo,
                    &mut info as *mut _ as *mut _,
                    mem::size_of::<FILE_STANDARD_INFO>() as u32,
                );

                if ret == 0 {
                    Err(Error::last_os_error())
                } else {
                    Ok(info.AllocationSize as u64)
                }
            }
        }
    };
}

macro_rules! allocate {
    ($file: ty) => {
        pub async fn allocate(file: &$file, len: u64) -> Result<()> {
            if allocated_size(file).await? < len {
                unsafe {
                    let mut info: FILE_ALLOCATION_INFO = mem::zeroed();
                    info.AllocationSize = len as i64;
                    let ret = SetFileInformationByHandle(
                        file.as_raw_handle() as HANDLE,
                        FileAllocationInfo,
                        &mut info as *mut _ as *mut _,
                        mem::size_of::<FILE_ALLOCATION_INFO>() as u32,
                    );
                    if ret == 0 {
                        return Err(Error::last_os_error());
                    }
                }
            }
            if file.metadata().await?.len() < len {
                file.set_len(len).await
            } else {
                Ok(())
            }
        }
    };
}

macro_rules! test_mod {
    ($annotation:meta, $($use_stmt:item)*) => {
        #[cfg(test)]
        mod test {
          extern crate tempdir;

          use crate::lock_contended_error;

          $(
              $use_stmt
          )*

          /// A file handle may not be exclusively locked multiple times, or exclusively locked and then
          /// shared locked.
          #[$annotation]
          async fn lock_non_reentrant() {
              let tempdir = tempdir::TempDir::new("fs4").unwrap();
              let path = tempdir.path().join("fs4");
              let file = fs::OpenOptions::new()
                  .read(true)
                  .write(true)
                  .create(true)
                  .open(&path)
                  .await
                  .unwrap();

              // Multiple exclusive locks fails.
              file.lock_exclusive().unwrap();
              assert_eq!(
                  file.try_lock_exclusive().unwrap_err().raw_os_error(),
                  lock_contended_error().raw_os_error()
              );
              file.unlock().unwrap();

              // Shared then Exclusive locks fails.
              file.lock_shared().unwrap();
              assert_eq!(
                  file.try_lock_exclusive().unwrap_err().raw_os_error(),
                  lock_contended_error().raw_os_error()
              );
          }

          /// A file handle can hold an exclusive lock and any number of shared locks, all of which must
          /// be unlocked independently.
          #[$annotation]
          async fn lock_layering() {
              let tempdir = tempdir::TempDir::new("fs4").unwrap();
              let path = tempdir.path().join("fs4");
              let file = fs::OpenOptions::new()
                  .read(true)
                  .write(true)
                  .create(true)
                  .open(&path)
                  .await
                  .unwrap();

              // Open two shared locks on the file, and then try and fail to open an exclusive lock.
              file.lock_exclusive().unwrap();
              file.lock_shared().unwrap();
              file.lock_shared().unwrap();
              assert_eq!(
                  file.try_lock_exclusive().unwrap_err().raw_os_error(),
                  lock_contended_error().raw_os_error()
              );

              // Pop one of the shared locks and try again.
              file.unlock().unwrap();
              assert_eq!(
                  file.try_lock_exclusive().unwrap_err().raw_os_error(),
                  lock_contended_error().raw_os_error()
              );

              // Pop the second shared lock and try again.
              file.unlock().unwrap();
              assert_eq!(
                  file.try_lock_exclusive().unwrap_err().raw_os_error(),
                  lock_contended_error().raw_os_error()
              );

              // Pop the exclusive lock and finally succeed.
              file.unlock().unwrap();
              file.lock_exclusive().unwrap();
          }

          /// A file handle with multiple open locks will have all locks closed on drop.
          #[$annotation]
          async fn lock_layering_cleanup() {
              let tempdir = tempdir::TempDir::new("fs4").unwrap();
              let path = tempdir.path().join("fs4");
              let file1 = fs::OpenOptions::new()
                  .read(true)
                  .write(true)
                  .create(true)
                  .open(&path)
                  .await
                  .unwrap();
              let file2 = fs::OpenOptions::new()
                  .read(true)
                  .write(true)
                  .create(true)
                  .open(&path)
                  .await
                  .unwrap();

              // Open two shared locks on the file, and then try and fail to open an exclusive lock.
              file1.lock_shared().unwrap();
              assert_eq!(
                  file2.try_lock_exclusive().unwrap_err().raw_os_error(),
                  lock_contended_error().raw_os_error()
              );

              drop(file1);
              file2.lock_exclusive().unwrap();
          }
        }
    };
}

cfg_async_std! {
    pub(crate) mod async_std_impl;
}

cfg_fs_err2_tokio! {
    pub(crate) mod fs_err2_tokio_impl;
}

cfg_fs_err3_tokio! {
    pub(crate) mod fs_err3_tokio_impl;
}

cfg_smol! {
    pub(crate) mod smol_impl;
}

cfg_tokio! {
    pub(crate) mod tokio_impl;
}
