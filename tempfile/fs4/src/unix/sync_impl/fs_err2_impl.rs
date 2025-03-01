use fs_err2::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;

lock_impl!(File);
allocate!(File);
allocate_size!(File);

test_mod! {
  use crate::fs_err2::FileExt;
  use fs_err2 as fs;
}
