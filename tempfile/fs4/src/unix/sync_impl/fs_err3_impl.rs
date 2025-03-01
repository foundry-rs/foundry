use fs_err3::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;

lock_impl!(File);
allocate!(File);
allocate_size!(File);

test_mod! {
  use crate::fs_err3::FileExt;
  use fs_err3 as fs;
}
