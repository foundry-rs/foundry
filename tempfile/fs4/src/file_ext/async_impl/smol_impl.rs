#[cfg(unix)]
use crate::unix::async_impl::smol_impl as sys;
#[cfg(windows)]
use crate::windows::async_impl::smol_impl as sys;
use smol::fs::File;

async_file_ext!(File, "smol::fs::File");

test_mod! {
  smol_potat::test,
  use crate::smol::AsyncFileExt;
  use smol::fs;
}
