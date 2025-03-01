use async_std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;

lock_impl!(File);
allocate!(File);
allocate_size!(File);

test_mod! {
  async_std::test,
  use crate::async_std::AsyncFileExt;
  use async_std::fs;
}
