//! Contains various tests related to `forge soldeer`.

use std::{
    fs::{self, OpenOptions},
    path::Path,
};

use foundry_test_utils::forgesoldeer;
use std::io::Write;
forgesoldeer!(install_dependency, |prj, cmd| {
    let command = "install";
    let dependency = "forge-std~1.8.1";

    let foundry_file = prj.root().join("foundry.toml");

    cmd.arg("soldeer").args([command, dependency]);
    cmd.execute();

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    let lock_contents = r#"
[[dependencies]]
name = "forge-std"
version = "1.8.1"
source = "https://soldeer-revisions.s3.amazonaws.com/forge-std/v1_8_1_23-03-2024_00:05:44_forge-std-v1.8.1.zip"
checksum = "0f7cd44f5670c31a9646d4031e70c66321cd3ed6ebac3c7278e4e57e4e5c5bd0"
"#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert_eq!(lock_contents, actual_lock_contents);

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"
# Full reference https://github.com/foundry-rs/foundry/tree/master/crates/config

[profile.default]
script = "script"
solc = "0.8.26"
src = "src"
test = "test"
libs = ["dependencies"]

[dependencies]
forge-std = "1.8.1"
"#;

    let actual_foundry_contents = read_file_to_string(&foundry_file);
    assert_eq!(foundry_contents, actual_foundry_contents);
});

forgesoldeer!(update_dependencies, |prj, cmd| {
    let command = "update";

    // We need to write this into the foundry.toml to make the update install the dependency
    let foundry_updates = r#"
[dependencies]
forge-std = { version = "1.8.1" }
"#;
    let foundry_file = prj.root().join("foundry.toml");

    let mut file = OpenOptions::new().append(true).open(&foundry_file).unwrap();

    if let Err(e) = write!(file, "{foundry_updates}") {
        eprintln!("Couldn't write to file: {e}");
    }

    cmd.arg("soldeer").arg(command);
    cmd.execute();

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    let lock_contents = r#"
[[dependencies]]
name = "forge-std"
version = "1.8.1"
source = "https://soldeer-revisions.s3.amazonaws.com/forge-std/v1_8_1_23-03-2024_00:05:44_forge-std-v1.8.1.zip"
checksum = "0f7cd44f5670c31a9646d4031e70c66321cd3ed6ebac3c7278e4e57e4e5c5bd0"
"#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert_eq!(lock_contents, actual_lock_contents);

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options

[dependencies]
forge-std = { version = "1.8.1" }
"#;

    let actual_foundry_contents = read_file_to_string(&foundry_file);
    assert_eq!(foundry_contents, actual_foundry_contents);
});

forgesoldeer!(update_dependencies_simple_version, |prj, cmd| {
    let command = "update";

    // We need to write this into the foundry.toml to make the update install the dependency, this
    // is he simplified version of version specification
    let foundry_updates = r#"
[dependencies]
forge-std = "1.8.1" 
"#;
    let foundry_file = prj.root().join("foundry.toml");

    let mut file = OpenOptions::new().append(true).open(&foundry_file).unwrap();

    if let Err(e) = write!(file, "{foundry_updates}") {
        eprintln!("Couldn't write to file: {e}");
    }

    cmd.arg("soldeer").arg(command);
    cmd.execute();

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    let lock_contents = r#"
[[dependencies]]
name = "forge-std"
version = "1.8.1"
source = "https://soldeer-revisions.s3.amazonaws.com/forge-std/v1_8_1_23-03-2024_00:05:44_forge-std-v1.8.1.zip"
checksum = "0f7cd44f5670c31a9646d4031e70c66321cd3ed6ebac3c7278e4e57e4e5c5bd0"
"#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert_eq!(lock_contents, actual_lock_contents);

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options

[dependencies]
forge-std = "1.8.1" 
"#;

    let actual_foundry_contents = read_file_to_string(&foundry_file);
    assert_eq!(foundry_contents, actual_foundry_contents);
});

forgesoldeer!(login, |prj, cmd| {
    let command = "login";

    cmd.arg("soldeer").arg(command);
    let output = cmd.unchecked_output();

    // On login, we can only check if the prompt is displayed in the stdout
    let stdout = String::from_utf8(output.stdout).expect("Could not parse the output");
    assert!(stdout.contains("Please enter your email"));
});

fn read_file_to_string(path: &Path) -> String {
    let contents: String = fs::read_to_string(path).unwrap_or_default();
    contents
}
