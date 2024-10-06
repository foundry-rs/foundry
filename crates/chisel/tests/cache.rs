use chisel::session::ChiselSession;
use foundry_compilers::artifacts::EvmVersion;
use foundry_config::{Config, SolcReq};
use semver::Version;
use serial_test::serial;
use std::path::Path;

#[test]
#[serial]
fn test_cache_directory() {
    // Get the cache dir
    // Should be ~/.foundry/cache/chisel
    let cache_dir = ChiselSession::cache_dir().unwrap();

    // Validate the cache directory
    let home_dir = dirs::home_dir().unwrap();
    assert_eq!(cache_dir, format!("{}/.foundry/cache/chisel/", home_dir.to_str().unwrap()));
}

#[test]
#[serial]
fn test_create_cache_directory() {
    // Get the cache dir
    let cache_dir = ChiselSession::cache_dir().unwrap();

    // Create the cache directory
    ChiselSession::create_cache_dir().unwrap();

    // Validate the cache directory
    assert!(Path::new(&cache_dir).exists());
}

#[test]
#[serial]
fn test_write_session() {
    // Create the cache directory if it doesn't exist
    let cache_dir = ChiselSession::cache_dir().unwrap();
    ChiselSession::create_cache_dir().unwrap();

    // Force the solc version to be 0.8.19
    let foundry_config = Config { evm_version: EvmVersion::London, ..Default::default() };

    // Create a new session
    let mut env = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config,
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("Failed to create ChiselSession!, {e}"));

    // Write the session
    let cached_session_name = env.write().unwrap();

    // Count the number of items in the cache_dir directory
    let mut num_items = std::fs::read_dir(&cache_dir).unwrap().count();
    num_items = if num_items > 0 { num_items - 1 } else { 0 };

    // Validate the session
    assert_eq!(cached_session_name, format!("{cache_dir}chisel-{num_items}.json"));
}

#[test]
#[serial]
fn test_write_session_with_name() {
    // Create the cache directory if it doesn't exist
    let cache_dir = ChiselSession::cache_dir().unwrap();
    ChiselSession::create_cache_dir().unwrap();

    // Force the solc version to be 0.8.19
    let foundry_config = Config { evm_version: EvmVersion::London, ..Default::default() };

    // Create a new session
    let mut env = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config,
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("Failed to create ChiselSession! {e}"));
    env.id = Some(String::from("test"));

    // Write the session
    let cached_session_name = env.write().unwrap();

    // Validate the session
    assert_eq!(cached_session_name, format!("{cache_dir}chisel-test.json"));
}

#[test]
#[serial]
fn test_clear_cache() {
    // Create a session to validate clearing a non-empty cache directory
    let cache_dir = ChiselSession::cache_dir().unwrap();

    // Force the solc version to be 0.8.19
    let foundry_config = Config { evm_version: EvmVersion::London, ..Default::default() };

    ChiselSession::create_cache_dir().unwrap();
    let mut env = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config,
        ..Default::default()
    })
    .unwrap_or_else(|_| panic!("Failed to create ChiselSession!"));
    env.write().unwrap();

    // Clear the cache
    ChiselSession::clear_cache().unwrap();

    // Validate there are no items in the cache dir
    let num_items = std::fs::read_dir(cache_dir).unwrap().count();
    assert_eq!(num_items, 0);
}

#[test]
#[serial]
fn test_list_sessions() {
    // Create and clear the cache directory
    ChiselSession::create_cache_dir().unwrap();
    ChiselSession::clear_cache().unwrap();

    // Force the solc version to be 0.8.19
    let foundry_config = Config { evm_version: EvmVersion::London, ..Default::default() };

    // Create a new session
    let mut env = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config,
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("Failed to create ChiselSession! {e}"));

    env.write().unwrap();

    // List the sessions
    let sessions = ChiselSession::list_sessions().unwrap();

    // Validate the sessions
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].1, "chisel-0.json");
}

#[test]
#[serial]
fn test_load_cache() {
    // Create and clear the cache directory
    ChiselSession::create_cache_dir().unwrap();
    ChiselSession::clear_cache().unwrap();

    // Force the solc version to be 0.8.19
    let foundry_config = Config { evm_version: EvmVersion::London, ..Default::default() };

    // Create a new session
    let mut env = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config,
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("Failed to create ChiselSession! {e}"));
    env.write().unwrap();

    // Load the session
    let new_env = ChiselSession::load("0");

    // Validate the session
    assert!(new_env.is_ok());
    let new_env = new_env.unwrap();
    assert_eq!(new_env.id.unwrap(), String::from("0"));
    assert_eq!(new_env.session_source.to_repl_source(), env.session_source.to_repl_source());
}

#[test]
#[serial]
fn test_write_same_session_multiple_times() {
    // Create and clear the cache directory
    ChiselSession::create_cache_dir().unwrap();
    ChiselSession::clear_cache().unwrap();

    // Force the solc version to be 0.8.19
    let foundry_config = Config { evm_version: EvmVersion::London, ..Default::default() };

    // Create a new session
    let mut env = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config,
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("Failed to create ChiselSession! {e}"));
    env.write().unwrap();
    env.write().unwrap();
    env.write().unwrap();
    env.write().unwrap();
    assert_eq!(ChiselSession::list_sessions().unwrap().len(), 1);
}

#[test]
#[serial]
fn test_load_latest_cache() {
    // Create and clear the cache directory
    ChiselSession::create_cache_dir().unwrap();
    ChiselSession::clear_cache().unwrap();

    // Force the solc version to be 0.8.19
    let foundry_config = Config { evm_version: EvmVersion::London, ..Default::default() };

    // Create sessions
    let mut env = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config: foundry_config.clone(),
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("Failed to create ChiselSession! {e}"));
    env.write().unwrap();

    let wait_time = std::time::Duration::from_millis(100);
    std::thread::sleep(wait_time);

    let mut env2 = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config,
        ..Default::default()
    })
    .unwrap_or_else(|e| panic!("Failed to create ChiselSession! {e}"));
    env2.write().unwrap();

    // Load the latest session
    let new_env = ChiselSession::latest().unwrap();

    // Validate the session
    assert_eq!(new_env.id.unwrap(), "1");
    assert_eq!(new_env.session_source.to_repl_source(), env.session_source.to_repl_source());
}

#[test]
#[serial]
fn test_solc_evm_configuration_mismatch() {
    // Create and clear the cache directory
    ChiselSession::create_cache_dir().unwrap();
    ChiselSession::clear_cache().unwrap();

    // Force the solc version to be 0.8.13 which does not support Paris
    let foundry_config = Config {
        evm_version: EvmVersion::Paris,
        solc: Some(SolcReq::Version(Version::new(0, 8, 13))),
        ..Default::default()
    };

    // Create a new session that is expected to fail
    let error = ChiselSession::new(chisel::session_source::SessionSourceConfig {
        foundry_config,
        ..Default::default()
    })
    .unwrap_err();

    assert_eq!(error.to_string(), "The set evm version, paris, is not supported by solc 0.8.13. Upgrade to a newer solc version.");
}
