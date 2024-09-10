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

    cmd.arg("soldeer").args([command, dependency]).assert_success().stdout_eq(str![[r#"
🦌 Running Soldeer install 🦌
No config file found. If you wish to proceed, please select how you want Soldeer to be configured:
1. Using foundry.toml
2. Using soldeer.toml
(Press 1 or 2), default is foundry.toml
Started HTTP download of forge-std~1.8.1
Dependency forge-std~1.8.1 downloaded!
Adding dependency forge-std-1.8.1 to the config file
The dependency forge-std~1.8.1 was unzipped!
Writing forge-std~1.8.1 to the lock file.
Added forge-std~1.8.1 to remappings

"#]]);

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    //     let lock_contents = r#"[[dependencies]]
    // name = "forge-std"
    // version = "1.8.1"
    // source = "https://soldeer-revisions.s3.amazonaws.com/forge-std/v1_8_1_23-03-2024_00:05:44_forge-std-v1.8.1.zip"
    // checksum = "0f7cd44f5670c31a9646d4031e70c66321cd3ed6ebac3c7278e4e57e4e5c5bd0"
    // integrity = "6a52f0c34d935e508af46a6d12a3a741798252f20a66f6bbee86c23dd6ef7c8d"
    // "#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    // assert_data_eq!(lock_contents, actual_lock_contents);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
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

    let foundry_file = prj.root().join("foundry.toml");

    cmd.arg("soldeer")
        .args([command, dependency, git])
        .assert_success()
        .stdout_eq(str![[r#"
🦌 Running Soldeer install 🦌
No config file found. If you wish to proceed, please select how you want Soldeer to be configured:
1. Using foundry.toml
2. Using soldeer.toml
(Press 1 or 2), default is foundry.toml
Started GIT download of forge-std~1.8.1
Successfully downloaded forge-std~1.8.1 the dependency via git
Dependency forge-std~1.8.1 downloaded!
Adding dependency forge-std-1.8.1 to the config file
Writing forge-std~1.8.1 to the lock file.
Added forge-std~1.8.1 to remappings

"#]])
        .stdout_eq(str![[r#"
🦌 Running Soldeer install 🦌
No config file found. If you wish to proceed, please select how you want Soldeer to be configured:
1. Using foundry.toml
2. Using soldeer.toml
(Press 1 or 2), default is foundry.toml
Started GIT download of forge-std~1.8.1
Successfully downloaded forge-std~1.8.1 the dependency via git
Dependency forge-std~1.8.1 downloaded!
Adding dependency forge-std-1.8.1 to the config file
Writing forge-std~1.8.1 to the lock file.
Added forge-std~1.8.1 to remappings

"#]]);

    // Making sure the path was created to the dependency and that README.md exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge = prj.root().join("dependencies").join("forge-std-1.8.1").join("README.md");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    //     let lock_contents = r#"[[dependencies]]
    // name = "forge-std"
    // version = "1.8.1"
    // source = "https://gitlab.com/mario4582928/Mario.git"
    // checksum = "22868f426bd4dd0e682b5ec5f9bd55507664240c"
    // "#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    // assert_data_eq!(lock_contents, actual_lock_contents);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
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

    let foundry_file = prj.root().join("foundry.toml");

    cmd.arg("soldeer")
        .args([command, dependency, git, rev_flag, commit])
        .assert_success()
        .stdout_eq(str![[r#"
🦌 Running Soldeer install 🦌
No config file found. If you wish to proceed, please select how you want Soldeer to be configured:
1. Using foundry.toml
2. Using soldeer.toml
(Press 1 or 2), default is foundry.toml
Started GIT download of forge-std~1.8.1
Successfully downloaded forge-std~1.8.1 the dependency via git
Dependency forge-std~1.8.1 downloaded!
Adding dependency forge-std-1.8.1 to the config file
Writing forge-std~1.8.1 to the lock file.
Added forge-std~1.8.1 to remappings

"#]]);

    // Making sure the path was created to the dependency and that README.md exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("JustATest2.md");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    //     let lock_contents = r#"[[dependencies]]
    // name = "forge-std"
    // version = "1.8.1"
    // source = "https://gitlab.com/mario4582928/Mario.git"
    // checksum = "7a0663eaf7488732f39550be655bad6694974cb3"
    // "#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    // assert_data_eq!(lock_contents, actual_lock_contents);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
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
    let foundry_updates = r#"
[dependencies]
"@tt" = {version = "1.6.1", url = "https://soldeer-revisions.s3.amazonaws.com/@openzeppelin-contracts/3_3_0-rc_2_22-01-2024_13:12:57_contracts.zip"}
forge-std = { version = "1.8.1" }
solmate = "6.7.0"
mario = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", rev = "22868f426bd4dd0e682b5ec5f9bd55507664240c" }
mario-custom-tag = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", tag = "custom-tag" }
mario-custom-branch = { version = "1.0", git = "https://gitlab.com/mario4582928/Mario.git", tag = "custom-branch" }
"#;
    let foundry_file = prj.root().join("foundry.toml");

    let mut file = OpenOptions::new().append(true).open(&foundry_file).unwrap();

    if let Err(e) = write!(file, "{foundry_updates}") {
        eprintln!("Couldn't write to file: {e}");
    }

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
    //     let lock_contents = r#"[[dependencies]]
    // name = "@tt"
    // version = "1.6.1"
    // source = "https://soldeer-revisions.s3.amazonaws.com/@openzeppelin-contracts/3_3_0-rc_2_22-01-2024_13:12:57_contracts.zip"
    // checksum = "3aa5b07e796ce2ae54bbab3a5280912444ae75807136a513fa19ff3a314c323f"
    // integrity = "24e7847580674bd0a4abf222b82fac637055141704c75a3d679f637acdcfe817"

    // [[dependencies]]
    // name = "forge-std"
    // version = "1.8.1"
    // source = "https://soldeer-revisions.s3.amazonaws.com/forge-std/v1_8_1_23-03-2024_00:05:44_forge-std-v1.8.1.zip"
    // checksum = "0f7cd44f5670c31a9646d4031e70c66321cd3ed6ebac3c7278e4e57e4e5c5bd0"
    // integrity = "6a52f0c34d935e508af46a6d12a3a741798252f20a66f6bbee86c23dd6ef7c8d"

    // [[dependencies]]
    // name = "mario"
    // version = "1.0"
    // source = "https://gitlab.com/mario4582928/Mario.git"
    // checksum = "22868f426bd4dd0e682b5ec5f9bd55507664240c"

    // [[dependencies]]
    // name = "mario-custom-branch"
    // version = "1.0"
    // source = "https://gitlab.com/mario4582928/Mario.git"
    // checksum = "84c3b38dba44a4c29ec44f45a31e1e59d36aa77b"

    // [[dependencies]]
    // name = "mario-custom-tag"
    // version = "1.0"
    // source = "https://gitlab.com/mario4582928/Mario.git"
    // checksum = "a366c4b560022d12e668d6c1756c6382e2352d0f"

    // [[dependencies]]
    // name = "solmate"
    // version = "6.7.0"
    // source = "https://soldeer-revisions.s3.amazonaws.com/solmate/6_7_0_22-01-2024_13:21:00_solmate.zip"
    // checksum = "dd0f08cdaaaad1de0ac45993d4959351ba89c2d9325a0b5df5570357064f2c33"
    // integrity = "ec330877af853f9d34b2b1bf692fb33c9f56450625f5c4abdcf0d3405839730e"
    // "#;

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
    let foundry_updates = r#"
[dependencies]
forge-std = "1.8.1" 
"#;
    let foundry_file = prj.root().join("foundry.toml");

    let mut file = OpenOptions::new().append(true).open(&foundry_file).unwrap();

    if let Err(e) = write!(file, "{foundry_updates}") {
        eprintln!("Couldn't write to file: {e}");
    }

    cmd.arg("soldeer").arg(command).assert_success().stdout_eq(str![[r#"
🦌 Running Soldeer update 🦌
Started HTTP download of forge-std~1.8.1
Dependency forge-std~1.8.1 downloaded!
The dependency forge-std~1.8.1 was unzipped!
Writing forge-std~1.8.1 to the lock file.

"#]]);

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    //     let lock_contents = r#"[[dependencies]]
    // name = "forge-std"
    // version = "1.8.1"
    // source = "https://soldeer-revisions.s3.amazonaws.com/forge-std/v1_8_1_23-03-2024_00:05:44_forge-std-v1.8.1.zip"
    // checksum = "0f7cd44f5670c31a9646d4031e70c66321cd3ed6ebac3c7278e4e57e4e5c5bd0"
    // integrity = "6a52f0c34d935e508af46a6d12a3a741798252f20a66f6bbee86c23dd6ef7c8d"
    // "#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    // assert_data_eq!(lock_contents, actual_lock_contents);
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

forgesoldeer!(login, |prj, cmd| {
    let command = "login";

    let output = cmd.arg("soldeer").arg(command).execute();

    // On login, we can only check if the prompt is displayed in the stdout
    let stdout = String::from_utf8(output.stdout).expect("Could not parse the output");
    assert!(stdout.contains("Please enter your email"));
});

forgesoldeer!(install_dependency_with_remappings_config, |prj, cmd| {
    let command = "install";
    let dependency = "forge-std~1.8.1";
    let foundry_updates = r#"
[soldeer]
remappings_generate = true
remappings_prefix = "@custom-f@"
remappings_location = "config"
remappings_regenerate = true
"#;
    let foundry_file = prj.root().join("foundry.toml");
    let mut file = OpenOptions::new().append(true).open(&foundry_file).unwrap();

    if let Err(e) = write!(file, "{foundry_updates}") {
        eprintln!("Couldn't write to file: {e}");
    }

    cmd.arg("soldeer").args([command, dependency]).assert_success().stdout_eq(str![[r#"
🦌 Running Soldeer install 🦌
No config file found. If you wish to proceed, please select how you want Soldeer to be configured:
1. Using foundry.toml
2. Using soldeer.toml
(Press 1 or 2), default is foundry.toml
Started HTTP download of forge-std~1.8.1
Dependency forge-std~1.8.1 downloaded!
Adding dependency forge-std-1.8.1 to the config file
The dependency forge-std~1.8.1 was unzipped!
Writing forge-std~1.8.1 to the lock file.
Added all dependencies to remapppings

"#]]);

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    //     let lock_contents = r#"[[dependencies]]
    // name = "forge-std"
    // version = "1.8.1"
    // source = "https://soldeer-revisions.s3.amazonaws.com/forge-std/v1_8_1_23-03-2024_00:05:44_forge-std-v1.8.1.zip"
    // checksum = "0f7cd44f5670c31a9646d4031e70c66321cd3ed6ebac3c7278e4e57e4e5c5bd0"
    // integrity = "6a52f0c34d935e508af46a6d12a3a741798252f20a66f6bbee86c23dd6ef7c8d"
    // "#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    // assert_data_eq!(lock_contents, actual_lock_contents);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let foundry_contents = r#"[profile.default]
src = "src"
out = "out"
libs = ["lib"]
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
[soldeer]
remappings_generate = true
remappings_prefix = "@custom-f@"
remappings_location = "txt"
remappings_regenerate = true
"#;
    let foundry_file = prj.root().join("foundry.toml");
    let mut file = OpenOptions::new().append(true).open(&foundry_file).unwrap();

    if let Err(e) = write!(file, "{foundry_updates}") {
        eprintln!("Couldn't write to file: {e}");
    }

    cmd.arg("soldeer").args([command, dependency]).assert_success().stdout_eq(str![[r#"
🦌 Running Soldeer install 🦌
No config file found. If you wish to proceed, please select how you want Soldeer to be configured:
1. Using foundry.toml
2. Using soldeer.toml
(Press 1 or 2), default is foundry.toml
Started HTTP download of forge-std~1.8.1
Dependency forge-std~1.8.1 downloaded!
Adding dependency forge-std-1.8.1 to the config file
The dependency forge-std~1.8.1 was unzipped!
Writing forge-std~1.8.1 to the lock file.
Added all dependencies to remapppings

"#]]);

    // Making sure the path was created to the dependency and that foundry.toml exists
    // meaning that the dependencies were installed correctly
    let path_dep_forge =
        prj.root().join("dependencies").join("forge-std-1.8.1").join("foundry.toml");
    assert!(path_dep_forge.exists());

    // Making sure the lock contents are the right ones
    let path_lock_file = prj.root().join("soldeer.lock");
    //     let lock_contents = r#"[[dependencies]]
    // name = "forge-std"
    // version = "1.8.1"
    // source = "https://soldeer-revisions.s3.amazonaws.com/forge-std/v1_8_1_23-03-2024_00:05:44_forge-std-v1.8.1.zip"
    // checksum = "0f7cd44f5670c31a9646d4031e70c66321cd3ed6ebac3c7278e4e57e4e5c5bd0"
    // integrity = "6a52f0c34d935e508af46a6d12a3a741798252f20a66f6bbee86c23dd6ef7c8d"
    // "#;

    let actual_lock_contents = read_file_to_string(&path_lock_file);
    // assert_data_eq!(lock_contents, actual_lock_contents);
    assert!(actual_lock_contents.contains("forge-std"));

    // Making sure the foundry contents are the right ones
    let remappings_content = r#"@custom-f@forge-std-1.8.1/=dependencies/forge-std-1.8.1/
"#;
    let remappings_file = prj.root().join("remappings.txt");
    assert_data_eq!(read_file_to_string(&remappings_file), remappings_content);
});

fn read_file_to_string(path: &Path) -> String {
    let contents: String = fs::read_to_string(path).unwrap_or_default();
    contents
}
