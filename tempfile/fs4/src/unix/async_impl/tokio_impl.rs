use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use tokio::fs::File;

lock_impl!(File);
allocate!(File);
allocate_size!(File);

test_mod! {
  tokio::test,
  use crate::tokio::AsyncFileExt;
  use tokio::fs;
}
