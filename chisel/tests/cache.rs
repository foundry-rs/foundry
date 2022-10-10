use std::path::Path;

use chisel::env::ChiselEnv;

#[test]
fn test_cache_directory() {
    // Get the cache dir
    // Should be ~/.foundry/cache/chisel
    let cache_dir = ChiselEnv::cache_dir().unwrap();
    println!("Cache dir: {:?}", cache_dir);

    // Validate the cache directory
    assert!(cache_dir.ends_with("/.foundry/cache/chisel/"));
}

#[test]
fn test_create_cache_directory() {
    // Get the cache dir
    let cache_dir = ChiselEnv::cache_dir().unwrap();
    println!("Gracefully creating cache dir: \"{:?}\"...", cache_dir);

    // Create the cache directory
    ChiselEnv::create_cache_dir().unwrap();

    // Validate the cache directory
    assert!(Path::new(&cache_dir).exists());
}

// #[test]
// fn test_get_latest_session() {
//     // Clean the cache directory
//     ChiselEnv::clean_cache_dir().unwrap();

//     // Get the latest session
//     // This should error since we cleaned the directory
//     let latest_session = ChiselEnv::latest_session().unwrap();
//     println!("Latest session: {:?}", latest_session);

//     // Validate the latest session
//     assert!(latest_session.ends_with("/.foundry/cache/chisel/latest/"));
// }
