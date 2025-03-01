#[cfg(unix)]
use crate::unix::async_impl::tokio_impl as sys;
#[cfg(windows)]
use crate::windows::async_impl::tokio_impl as sys;
use tokio::fs::File;

async_file_ext!(File, "tokio::fs::File");

test_mod! {
  tokio::test,
  use crate::tokio::AsyncFileExt;
  use tokio::fs;
}
