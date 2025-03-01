macro_rules! allocate {
    ($file: ty) => {
        #[cfg(any(
            target_os = "aix",
            target_os = "linux",
            target_os = "freebsd",
            target_os = "android",
            target_os = "emscripten",
            target_os = "nacl",
            target_os = "macos",
            target_os = "ios",
            target_os = "watchos",
            target_os = "tvos"
        ))]
        pub async fn allocate(file: &$file, len: u64) -> std::io::Result<()> {
            use rustix::{
                fd::BorrowedFd,
                fs::{fallocate, FallocateFlags},
            };
            unsafe {
                let borrowed_fd = BorrowedFd::borrow_raw(file.as_raw_fd());
                match fallocate(borrowed_fd, FallocateFlags::empty(), 0, len) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(std::io::Error::from_raw_os_error(e.raw_os_error())),
                }
            }
        }

        #[cfg(any(
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly",
            target_os = "solaris",
            target_os = "illumos",
            target_os = "haiku",
            target_os = "hurd",
        ))]
        pub async fn allocate(file: &$file, len: u64) -> std::io::Result<()> {
            // No file allocation API available, just set the length if necessary.
            if len > file.metadata().await?.len() as u64 {
                file.set_len(len).await
            } else {
                Ok(())
            }
        }
    };
}

macro_rules! allocate_size {
    ($file: ty) => {
        pub async fn allocated_size(file: &$file) -> std::io::Result<u64> {
            file.metadata().await.map(|m| m.blocks() as u64 * 512)
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

          /// Tests that locking a file descriptor will replace any existing locks
          /// held on the file descriptor.
          #[$annotation]
          async fn lock_replace() {
              let tempdir = tempdir::TempDir::new("fs4").unwrap();
              let path = tempdir.path().join("fs4");
              let file1 = fs::OpenOptions::new()
                  .write(true)
                  .create(true)
                  .truncate(true)
                  .open(&path)
                  .await
                  .unwrap();
              let file2 = fs::OpenOptions::new()
                  .write(true)
                  .create(true)
                  .truncate(true)
                  .open(&path)
                  .await
                  .unwrap();

              // Creating a shared lock will drop an exclusive lock.
              file1.lock_exclusive().unwrap();
              file1.lock_shared().unwrap();
              file2.lock_shared().unwrap();

              // Attempting to replace a shared lock with an exclusive lock will fail
              // with multiple lock holders, and remove the original shared lock.
              assert_eq!(
                  file2.try_lock_exclusive().unwrap_err().raw_os_error(),
                  lock_contended_error().raw_os_error()
              );
              file1.lock_shared().unwrap();
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
