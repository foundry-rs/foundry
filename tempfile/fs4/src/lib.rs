//! Extended utilities for working with files and filesystems in Rust.
#![doc(html_root_url = "https://docs.rs/fs4/0.12.0")]
#![cfg_attr(test, feature(test))]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, allow(unused_attributes))]
#![allow(unexpected_cfgs, unstable_name_collisions)]

#[cfg(windows)]
extern crate windows_sys;

macro_rules! cfg_async_std {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "async-std")]
            #[cfg_attr(docsrs, doc(cfg(feature = "async-std")))]
            $item
        )*
    }
}

// This lint is a bug, it is being used in multiple places.
#[allow(unused_macros)]
macro_rules! cfg_fs_err2 {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "fs-err2")]
            #[cfg_attr(docsrs, doc(cfg(feature = "fs-err2")))]
            $item
        )*
    }
}

macro_rules! cfg_fs_err2_tokio {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "fs-err2-tokio")]
            #[cfg_attr(docsrs, doc(cfg(feature = "fs-err2-tokio")))]
            $item
        )*
    }
}

// This lint is a bug, it is being used in multiple places.
#[allow(unused_macros)]
macro_rules! cfg_fs_err3 {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "fs-err3")]
            #[cfg_attr(docsrs, doc(cfg(feature = "fs-err3")))]
            $item
        )*
    }
}

macro_rules! cfg_fs_err3_tokio {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "fs-err3-tokio")]
            #[cfg_attr(docsrs, doc(cfg(feature = "fs-err3-tokio")))]
            $item
        )*
    }
}

macro_rules! cfg_smol {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "smol")]
            #[cfg_attr(docsrs, doc(cfg(feature = "smol")))]
            $item
        )*
    }
}

macro_rules! cfg_tokio {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "tokio")]
            #[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
            $item
        )*
    }
}

macro_rules! cfg_sync {
  ($($item:item)*) => {
      $(
          #[cfg(feature = "sync")]
          #[cfg_attr(docsrs, doc(cfg(feature = "sync")))]
          $item
      )*
  }
}

macro_rules! cfg_fs2_err {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "fs-err2")]
            #[cfg_attr(docsrs, doc(cfg(feature = "fs-err2")))]
            $item
        )*
    }
}

macro_rules! cfg_fs3_err {
    ($($item:item)*) => {
        $(
            #[cfg(feature = "fs-err3")]
            #[cfg_attr(docsrs, doc(cfg(feature = "fs-err3")))]
            $item
        )*
    }
}

macro_rules! cfg_async {
    ($($item:item)*) => {
        $(
            #[cfg(any(feature = "smol", feature = "async-std", feature = "tokio", feature = "fs-err-tokio"))]
            #[cfg_attr(docsrs, doc(cfg(any(feature = "smol", feature = "async-std", feature = "tokio", feature = "fs-err-tokio"))))]
            $item
        )*
    }
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
use windows as sys;

mod file_ext;

cfg_sync!(
    pub mod fs_std {
        pub use crate::file_ext::sync_impl::std_impl::FileExt;
    }
);

cfg_fs_err2!(
    pub mod fs_err2 {
        pub use crate::file_ext::sync_impl::fs_err2_impl::FileExt;
    }
);

cfg_fs_err3!(
    pub mod fs_err3 {
        pub use crate::file_ext::sync_impl::fs_err3_impl::FileExt;
    }
);

cfg_async_std!(
    pub mod async_std {
        pub use crate::file_ext::async_impl::async_std_impl::AsyncFileExt;
    }
);

cfg_fs_err2_tokio!(
    pub mod fs_err2_tokio {
        pub use crate::file_ext::async_impl::fs_err2_tokio_impl::AsyncFileExt;
    }
);

cfg_fs_err3_tokio!(
    pub mod fs_err3_tokio {
        pub use crate::file_ext::async_impl::fs_err3_tokio_impl::AsyncFileExt;
    }
);

cfg_smol!(
    pub mod smol {
        pub use crate::file_ext::async_impl::smol_impl::AsyncFileExt;
    }
);

cfg_tokio!(
    pub mod tokio {
        pub use crate::file_ext::async_impl::tokio_impl::AsyncFileExt;
    }
);

mod fs_stats;
pub use fs_stats::FsStats;

use std::io::{Error, Result};
use std::path::Path;

/// Returns the error that a call to a try lock method on a contended file will
/// return.
pub fn lock_contended_error() -> Error {
    sys::lock_error()
}

/// Get the stats of the file system containing the provided path.
pub fn statvfs<P>(path: P) -> Result<FsStats>
where
    P: AsRef<Path>,
{
    sys::statvfs(path.as_ref())
}

/// Returns the number of free bytes in the file system containing the provided
/// path.
pub fn free_space<P>(path: P) -> Result<u64>
where
    P: AsRef<Path>,
{
    statvfs(path).map(|stat| stat.free_space)
}

/// Returns the available space in bytes to non-priveleged users in the file
/// system containing the provided path.
pub fn available_space<P>(path: P) -> Result<u64>
where
    P: AsRef<Path>,
{
    statvfs(path).map(|stat| stat.available_space)
}

/// Returns the total space in bytes in the file system containing the provided
/// path.
pub fn total_space<P>(path: P) -> Result<u64>
where
    P: AsRef<Path>,
{
    statvfs(path).map(|stat| stat.total_space)
}

/// Returns the filesystem's disk space allocation granularity in bytes.
/// The provided path may be for any file in the filesystem.
///
/// On Posix, this is equivalent to the filesystem's block size.
/// On Windows, this is equivalent to the filesystem's cluster size.
pub fn allocation_granularity<P>(path: P) -> Result<u64>
where
    P: AsRef<Path>,
{
    statvfs(path).map(|stat| stat.allocation_granularity)
}
