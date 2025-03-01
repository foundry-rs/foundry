#[cfg(unix)]
use crate::unix::sync_impl::std_impl as sys;
#[cfg(windows)]
use crate::windows::sync_impl::std_impl as sys;
use std::fs::File;

file_ext!(File, "std::fs::File");

test_mod! {
  use std::fs;
}
