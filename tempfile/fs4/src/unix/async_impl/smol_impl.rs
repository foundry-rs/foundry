use smol::fs::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;

lock_impl!(File);
allocate!(File);
allocate_size!(File);

test_mod! {
  smol_potat::test,
  use crate::smol::AsyncFileExt;
  use smol::fs;
}
