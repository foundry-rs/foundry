#[cfg(unix)]
use crate::unix::sync_impl::fs_err3_impl as sys;
#[cfg(windows)]
use crate::windows::sync_impl::fs_err3_impl as sys;
use fs_err3::File;

file_ext!(File, "fs_err::File");

test_mod! {
  use fs_err3 as fs;
}
