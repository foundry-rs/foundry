#[cfg(unix)]
use crate::unix::async_impl::async_std_impl as sys;
#[cfg(windows)]
use crate::windows::async_impl::async_std_impl as sys;
use async_std::fs::File;

async_file_ext!(File, "async_std::fs::File");

test_mod! {
  async_std::test,
  use crate::async_std::AsyncFileExt;
  use async_std::fs;
}
