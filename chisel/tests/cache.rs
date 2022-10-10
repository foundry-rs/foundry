use std::path::Path;
use crate::env::ChiselEnv;

#[test]
fn test_cache_directory() {
    // Get the cache dir
    // Should be ~/.chisel
    let cache_dir = ChiselEnv::cache_dir().unwrap();

    // Validate the cache directory
    assert!(Path::new(&cache_dir).exists());
    // assert!(std::str::ends_with(&cache_dir, "/.chisel/"));
    // assert!(std:: cache_dir , PathBuf::from("/home/runner/.chisel"));
}