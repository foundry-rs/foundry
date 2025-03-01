macro_rules! async_file_ext {
    ($file: ty, $file_name: literal) => {
        use std::io::Result;

        #[doc = concat!("Extension trait for `", $file_name, "` which provides allocation, duplication and locking methods.")]
        ///
        /// ## Notes on File Locks
        ///
        /// This library provides whole-file locks in both shared (read) and exclusive
        /// (read-write) varieties.
        ///
        /// File locks are a cross-platform hazard since the file lock APIs exposed by
        /// operating system kernels vary in subtle and not-so-subtle ways.
        ///
        /// The API exposed by this library can be safely used across platforms as long
        /// as the following rules are followed:
        ///
        ///   * Multiple locks should not be created on an individual `File` instance
        ///     concurrently.
        ///   * Duplicated files should not be locked without great care.
        ///   * Files to be locked should be opened with at least read or write
        ///     permissions.
        ///   * File locks may only be relied upon to be advisory.
        ///
        /// See the tests in `lib.rs` for cross-platform lock behavior that may be
        /// relied upon; see the tests in `unix.rs` and `windows.rs` for examples of
        /// platform-specific behavior. File locks are implemented with
        /// [`flock(2)`](http://man7.org/linux/man-pages/man2/flock.2.html) on Unix and
        /// [`LockFile`](https://msdn.microsoft.com/en-us/library/windows/desktop/aa365202(v=vs.85).aspx)
        /// on Windows.
        pub trait AsyncFileExt {
            /// Returns the amount of physical space allocated for a file.
            fn allocated_size(&self) -> impl core::future::Future<Output = Result<u64>>;

            /// Ensures that at least `len` bytes of disk space are allocated for the
            /// file, and the file size is at least `len` bytes. After a successful call
            /// to `allocate`, subsequent writes to the file within the specified length
            /// are guaranteed not to fail because of lack of disk space.
            fn allocate(&self, len: u64) -> impl core::future::Future<Output = Result<()>>;

            /// Locks the file for shared usage, blocking if the file is currently
            /// locked exclusively.
            fn lock_shared(&self) -> Result<()>;

            /// Locks the file for exclusive usage, blocking if the file is currently
            /// locked.
            fn lock_exclusive(&self) -> Result<()>;

            /// Locks the file for shared usage, or returns an error if the file is
            /// currently locked (see `lock_contended_error`).
            fn try_lock_shared(&self) -> Result<()>;

            /// Locks the file for exclusive usage, or returns an error if the file is
            /// currently locked (see `lock_contended_error`).
            fn try_lock_exclusive(&self) -> Result<()>;

            /// Unlocks the file.
            fn unlock(&self) -> Result<()>;

            /// Unlocks the file.
            ///
            /// **Note:** This method is not really "async", the underlying system call is still blocking.
            /// Having this method as async is just for convenience when using it in async runtime.
            fn unlock_async(&self) -> impl core::future::Future<Output = Result<()>>;
        }

        impl AsyncFileExt for $file {
            async fn allocated_size(&self) -> Result<u64> {
                sys::allocated_size(self).await
            }
            async fn allocate(&self, len: u64) -> Result<()> {
                sys::allocate(self, len).await
            }
            fn lock_shared(&self) -> Result<()> {
                sys::lock_shared(self)
            }

            fn lock_exclusive(&self) -> Result<()> {
                sys::lock_exclusive(self)
            }

            fn try_lock_shared(&self) -> Result<()> {
                sys::try_lock_shared(self)
            }

            fn try_lock_exclusive(&self) -> Result<()> {
                sys::try_lock_exclusive(self)
            }

            fn unlock(&self) -> Result<()> {
                sys::unlock(self)
            }

            async fn unlock_async(&self) -> Result<()> {
                sys::unlock(self)
            }
        }
    }
}

macro_rules! test_mod {
     ($annotation:meta, $($use_stmt:item)*) => {
        #[cfg(test)]
        mod test {
            extern crate tempdir;
            extern crate test;
            use crate::{
                allocation_granularity, available_space, free_space,
                lock_contended_error, total_space,
            };

            $(
                $use_stmt
            )*

            /// Tests shared file lock operations.
            #[$annotation]
            async fn lock_shared() {
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
                let file3 = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open(&path)
                    .await
                    .unwrap();

                // Concurrent shared access is OK, but not shared and exclusive.
                file1.lock_shared().unwrap();
                file2.lock_shared().unwrap();
                assert_eq!(
                    file3.try_lock_exclusive().unwrap_err().kind(),
                    lock_contended_error().kind()
                );
                file1.unlock().unwrap();
                assert_eq!(
                    file3.try_lock_exclusive().unwrap_err().kind(),
                    lock_contended_error().kind()
                );

                // Once all shared file locks are dropped, an exclusive lock may be created;
                file2.unlock().unwrap();
                file3.lock_exclusive().unwrap();
            }

            /// Tests exclusive file lock operations.
            #[$annotation]
            async fn lock_exclusive() {
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

                // No other access is possible once an exclusive lock is created.
                file1.lock_exclusive().unwrap();
                assert_eq!(
                    file2.try_lock_exclusive().unwrap_err().kind(),
                    lock_contended_error().kind()
                );
                assert_eq!(
                    file2.try_lock_shared().unwrap_err().kind(),
                    lock_contended_error().kind()
                );

                // Once the exclusive lock is dropped, the second file is able to create a lock.
                file1.unlock().unwrap();
                file2.lock_exclusive().unwrap();
            }

            /// Tests that a lock is released after the file that owns it is dropped.
            #[$annotation]
            async fn lock_cleanup() {
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

                file1.lock_exclusive().unwrap();
                assert_eq!(
                    file2.try_lock_shared().unwrap_err().kind(),
                    lock_contended_error().kind()
                );

                // Drop file1; the lock should be released.
                drop(file1);
                file2.lock_shared().unwrap();
            }

            /// Tests file allocation.
            #[$annotation]
            async fn allocate() {
                let tempdir = tempdir::TempDir::new("fs4").unwrap();
                let path = tempdir.path().join("fs4");
                let file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .open(&path)
                    .await
                    .unwrap();
                let blksize = allocation_granularity(&path).unwrap();

                // New files are created with no allocated size.
                assert_eq!(0, file.allocated_size().await.unwrap());
                assert_eq!(0, file.metadata().await.unwrap().len());

                // Allocate space for the file, checking that the allocated size steps
                // up by block size, and the file length matches the allocated size.

                file.allocate(2 * blksize - 1).await.unwrap();
                assert_eq!(2 * blksize, file.allocated_size().await.unwrap());
                assert_eq!(2 * blksize - 1, file.metadata().await.unwrap().len());

                // Truncate the file, checking that the allocated size steps down by
                // block size.

                file.set_len(blksize + 1).await.unwrap();
                assert_eq!(2 * blksize, file.allocated_size().await.unwrap());
                assert_eq!(blksize + 1, file.metadata().await.unwrap().len());
            }

            /// Checks filesystem space methods.
            #[$annotation]
            async fn filesystem_space() {
                let tempdir = tempdir::TempDir::new("fs4").unwrap();
                let total_space = total_space(tempdir.path()).unwrap();
                let free_space = free_space(tempdir.path()).unwrap();
                let available_space = available_space(tempdir.path()).unwrap();

                assert!(total_space > free_space);
                assert!(total_space > available_space);
                assert!(available_space <= free_space);
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
