//! Contains various tests related to `forge soldeer`.

use std::{fs, path::Path};

use foundry_test_utils::forgesoldeer;
forgesoldeer!(install_dependency, |prj, cmd| {
    let command = "install";
    let dependency = "forge-std~1.8.1";

    let mut foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[dependencies]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options
"#;
    let foundry_file = prj.root().join("foundry.toml");
    fs::write(&foundry_file, foundry_contents).unwrap();

    cmd.arg("soldeer").args([command, dependency]).assert_success();

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[dependencies]
forge-std = "1.8.1"

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options
"#;

    assert_data_eq!(read_file_to_string(&foundry_file), foundry_contents);
});

forgesoldeer!(install_dependency_git, |prj, cmd| {
    let command = "install";
    let dependency = "forge-std~1.8.1";
    let git = "https://gitlab.com/mario4582928/Mario.git";

    let mut foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[dependencies]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options
"#;
    let foundry_file = prj.root().join("foundry.toml");
    fs::write(&foundry_file, foundry_contents).unwrap();

    cmd.arg("soldeer").args([command, dependency, git]).assert_success();

    // Making sure the path was created to the dependency and that README.md exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge = prj.root().join("dependencies").join("forge-std-1.8.1").join("README.md");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[dependencies]
forge-std = { version = "1.8.1", git = "https://gitlab.com/mario4582928/Mario.git", rev = "22868f426bd4dd0e682b5ec5f9bd55507664240c" }

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options
"#;

    assert_data_eq!(read_file_to_string(&foundry_file), foundry_contents);
});

forgesoldeer!(install_dependency_git_commit, |prj, cmd| {
    let command = "install";
    let dependency = "forge-std~1.8.1";
    let git = "https://gitlab.com/mario4582928/Mario.git";
    let rev_flag = "--rev";
    let commit = "7a0663eaf7488732f39550be655bad6694974cb3";

    let mut foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[dependencies]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options
"#;
    let foundry_file = prj.root().join("foundry.toml");
    fs::write(&foundry_file, foundry_contents).unwrap();

    cmd.arg("soldeer").args([command, dependency, git, rev_flag, commit]).assert_success();

    // Making sure the path was created to the dependency and that README.md exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("JustATest2.md");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[dependencies]
forge-std = { version = "1.8.1", git = "https://gitlab.com/mario4582928/Mario.git", rev = "7a0663eaf7488732f39550be655bad6694974cb3" }

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options
"#;

    assert_data_eq!(read_file_to_string(&foundry_file), foundry_contents);
});

forgesoldeer!(update_dependencies, |prj, cmd| {
    let command = "update";

    // We need to write this into the foundry.toml to make the update install the dependency
    let foundry_updates = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options

[dependencies]
"@tt" = {version = "1.6.1", url = "https://soldeer-revisions.s3.amazonaws.com/@openzeppelin-contracts/3_3_0-rc_2_22-01-2024_13:12:57_contracts.zip"}
forge-std = { version = "1.8.1" }
solmate = "6.7.0"
mario = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", rev = "22868f426bd4dd0e682b5ec5f9bd55507664240c" }
mario-custom-tag = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", tag = "custom-tag" }
mario-custom-branch = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", tag = "custom-branch" }
"#;
    let foundry_file = prj.root().join("foundry.toml");
    fs::write(&foundry_file, foundry_updates).unwrap();

    cmd.arg("soldeer").arg(command).assert_success();

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let dep1 = prj.root().join("dependencies").join("@tt-1.6.1");
    let dep2 = prj.root().join("dependencies").join("forge-std-1.8.1");
    let dep3 = prj.root().join("dependencies").join("mario-1.0");
    let dep4 = prj.root().join("dependencies").join("solmate-6.7.0");
    let dep5 = prj.root().join("dependencies").join("mario-custom-tag-1.0");
    let dep6 = prj.root().join("dependencies").join("mario-custom-branch-1.0");

    assert!(dep1.exists());
    assert!(dep2.exists());
    assert!(dep3.exists());
    assert!(dep4.exists());
    assert!(dep5.exists());
    assert!(dep6.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");

    // assert_data_eq!(lock_contents, read_file_to_string(&path_lock_file));
    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options

[dependencies]
"@tt" = {version = "1.6.1", url = "https://soldeer-revisions.s3.amazonaws.com/@openzeppelin-contracts/3_3_0-rc_2_22-01-2024_13:12:57_contracts.zip"}
forge-std = { version = "1.8.1" }
solmate = "6.7.0"
mario = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", rev = "22868f426bd4dd0e682b5ec5f9bd55507664240c" }
mario-custom-tag = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", tag = "custom-tag" }
mario-custom-branch = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", tag = "custom-branch" }
"#;
    assert_data_eq!(read_file_to_string(&foundry_file), foundry_contents);
});

forgesoldeer!(update_dependencies_simple_version, |prj, cmd| {
    let command = "update";

    // We need to write this into the foundry.toml to make the update install the dependency, this
    // is he simplified version of version specification
    let foundry_updates = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options

[dependencies]
forge-std = "1.8.1" 
"#;
    let foundry_file = prj.root().join("foundry.toml");

    fs::write(&foundry_file, foundry_updates).unwrap();

    cmd.arg("soldeer").arg(command).assert_success();
    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options

[dependencies]
forge-std = "1.8.1" 
"#;

    assert_data_eq!(read_file_to_string(&foundry_file), foundry_contents);
});

forgesoldeer!(install_dependency_with_remappings_config, |prj, cmd| {
    let command = "install";
    let dependency = "forge-std~1.8.1";
    let foundry_updates = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib", "dependencies"]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options

[soldeer]
remappings_generate = true
remappings_prefix = "@custom-f@"
remappings_location = "config"
remappings_regenerate = true

[dependencies]
"#;
    let foundry_file = prj.root().join("foundry.toml");
    fs::write(&foundry_file, foundry_updates).unwrap();

    cmd.arg("soldeer").args([command, dependency]).assert_success();

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib", "dependencies"]
remappings = ["@custom-f@forge-std-1.8.1/=dependencies/forge-std-1.8.1/"]

# See more config options https://github.com/foundry-rs/foundry/blob/master/crates/config/README.md#all-options

[soldeer]
remappings_generate = true
remappings_prefix = "@custom-f@"
remappings_location = "config"
remappings_regenerate = true

[dependencies]
forge-std = "1.8.1"
"#;

    assert_data_eq!(read_file_to_string(&foundry_file), foundry_contents);
});

forgesoldeer!(install_dependency_with_remappings_txt, |prj, cmd| {
    let command = "install";
    let dependency = "forge-std~1.8.1";
    let foundry_updates = r#"
[dependencies]

[soldeer]
remappings_generate = true
remappings_prefix = "@custom-f@"
remappings_location = "txt"
remappings_regenerate = true
"#;
    let foundry_file = prj.root().join("foundry.toml");
    fs::write(&foundry_file, foundry_updates).unwrap();

    cmd.arg("soldeer").args([command, dependency]).assert_success();

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let remappings_content = r#"@custom-f@forge-std-1.8.1/=dependencies/forge-std-1.8.1/
"#;
    let remappings_file = prj.root().join("remappings.txt");
    assert_data_eq!(read_file_to_string(&remappings_file), remappings_content);
});

forgesoldeer!(login, |prj, cmd| {
    let command = "login";

    let _ = cmd.arg("soldeer").arg(command).assert_failure();
});

fn read_file_to_string(path: &Path) -> String {
    let contents: String = fs::read_to_string(path).unwrap_or_default();
    contents
}
