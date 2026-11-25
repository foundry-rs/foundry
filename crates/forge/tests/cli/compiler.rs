//! Tests for the `forge compiler` command.

use foundry_compilers::compilers::solc::Solc;
use foundry_test_utils::{
    snapbox::IntoData,
    util::{SOLC_VERSION, get_vyper},
};
use path_slash::PathBufExt;
use std::fs;

const CONTRACT_A: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.4;

contract ContractA {}
"#;

const CONTRACT_B: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.11;

contract ContractB {}
"#;

const CONTRACT_C: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.33;

contract ContractC {}
"#;

const CONTRACT_D: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.33;

contract ContractD {}
"#;

const VYPER_INTERFACE: &str = r#"
# pragma version >=0.4.0

@external
@view
def number() -> uint256:
    return empty(uint256)

@external
def set_number(new_number: uint256):
    pass

@external
def increment() -> uint256:
    return empty(uint256)
"#;

const VYPER_CONTRACT: &str = r#"
import ICounter
implements: ICounter

number: public(uint256)

@external
def set_number(new_number: uint256):
    self.number = new_number

@external
def increment() -> uint256:
    self.number += 1
    return self.number
"#;

#[forgetest]
fn can_resolve_path(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);

    cmd.args(["compiler", "resolve", "--root", prj.root().to_str().unwrap()])
        .assert_success()
        .stdout_eq(str![[r#"
Solidity:
- 0.8.4


"#]]);
}

#[forgetest]
fn can_list_resolved_compiler_versions(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Solidity:
- 0.8.4


"#]]);
}

#[forgetest_init]
fn can_print_resolved_compiler_path(prj: _, cmd: _) {
    prj.add_source("Contract", "contract Contract {}");
    let solc = Solc::find_svm_installed_version(&SOLC_VERSION.parse().unwrap()).unwrap().unwrap();

    cmd.args(["compiler", "resolve", "--path"])
        .assert_success()
        .stdout_eq(format!("{}\n", solc.solc.to_slash_lossy()));
}

#[forgetest]
fn can_print_resolved_vyper_path(prj: _, cmd: _) {
    let vyper = get_vyper();
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);
    prj.update_config(|config| config.vyper.path = Some(vyper.path.clone()));

    cmd.args(["compiler", "resolve", "--path"])
        .assert_success()
        .stdout_eq(format!("{}\n", vyper.path.display()));
}

#[forgetest]
fn compiler_path_requires_single_version(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);

    cmd.args(["compiler", "resolve", "--path"]).assert_failure().stdout_eq("").stderr_eq(
        "Error: multiple compilers resolved; use `forge compiler resolve` to inspect them\n",
    );
}

#[forgetest]
fn can_list_resolved_compiler_versions_json(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);

    cmd.args(["compiler", "resolve", "--json"]).assert_success().stdout_eq(
        str![[r#"
{
   "Solidity":[
      {
         "version":"0.8.4"
      }
   ]
}
"#]]
        .is_json(),
    );
}

#[forgetest]
fn can_list_resolved_compiler_versions_verbose(prj: _, cmd: _) {
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);

    cmd.args(["compiler", "resolve", "-v"]).assert_success().stdout_eq(str![[r#"
Solidity:

0.8.33:
├── src/ContractC.sol
└── src/ContractD.sol


"#]]);
}

#[forgetest]
fn can_list_resolved_compiler_versions_verbose_json(prj: _, cmd: _) {
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);

    cmd.args(["compiler", "resolve", "--json", "-v"]).assert_success().stdout_eq(
        str![[r#"
{
  "Solidity": [
    {
      "version": "0.8.33",
      "paths": [
        "src/ContractC.sol",
        "src/ContractD.sol"
      ]
    }
  ]
}
"#]]
        .is_json(),
    );
}

#[forgetest]
fn can_list_resolved_multiple_compiler_versions(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Solidity:
- 0.8.4
- 0.8.11
- 0.8.33

Vyper:
- 0.4.3


"#]]);
}

#[forgetest]
fn can_list_resolved_multiple_compiler_versions_skipped(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve", "--skip", ".sol", "-v"]).assert_success().stdout_eq(str![[
        r#"
Vyper:

0.4.3:
├── src/Counter.vy
└── src/ICounter.vyi


"#
    ]]);
}

#[forgetest]
fn can_list_resolved_multiple_compiler_versions_skipped_json(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve", "--skip", "Contract(A|B|C)", "--json", "-v"])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
  "Solidity": [
    {
      "version": "0.8.33",
      "paths": [
        "src/ContractD.sol"
      ]
    }
  ],
  "Vyper": [
    {
      "version": "0.4.3",
      "paths": [
        "src/Counter.vy",
        "src/ICounter.vyi"
      ]
    }
  ]
}
"#]]
            .is_json(),
        );
}

#[forgetest]
fn can_list_resolved_multiple_compiler_versions_verbose(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve", "-vv"]).assert_success().stdout_eq(str![[r#"
Solidity:

0.8.4 (<= istanbul):
└── src/ContractA.sol

0.8.11 (<= london):
└── src/ContractB.sol

0.8.33 (<= prague):
├── src/ContractC.sol
└── src/ContractD.sol

Vyper:

0.4.3 (<= prague):
├── src/Counter.vy
└── src/ICounter.vyi


"#]]);
}

#[forgetest]
fn can_list_resolved_multiple_compiler_versions_verbose_json(prj: _, cmd: _) {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);

    cmd.args(["compiler", "resolve", "--json", "-vv"]).assert_success().stdout_eq(
        str![[r#"
{
  "Solidity": [
    {
      "version": "0.8.4",
      "evm_version": "Istanbul",
      "paths": [
        "src/ContractA.sol"
      ]
    },
    {
      "version": "0.8.11",
      "evm_version": "London",
      "paths": [
        "src/ContractB.sol"
      ]
    },
    {
      "version": "0.8.33",
      "evm_version": "[..]",
      "paths": [
        "src/ContractC.sol",
        "src/ContractD.sol"
      ]
    }
  ],
  "Vyper": [
    {
      "version": "0.4.3",
      "evm_version": "[..]",
      "paths": [
        "src/Counter.vy",
        "src/ICounter.vyi"
      ]
    }
  ]
}
"#]]
        .is_json(),
    );
}

#[forgetest]
fn compiler_install_rejects_invalid_solar_pin(cmd: _) {
    cmd.args(["compiler", "install", "solar:0.1.8"]).assert_failure().stderr_eq(str![[r#"
error: invalid value 'solar:0.1.8' for '<COMPILER>...': expected a solc version such as `0.8.30`, `solar`, or `solar:<version>` naming a release 0.2.0 or later

For more information, try '--help'.

"#]]);
}

#[cfg(unix)]
#[forgetest]
fn can_install_list_and_remove_solar(cmd: _) {
    let home = tempfile::tempdir().unwrap();
    let home = home.path().to_str().unwrap();

    cmd.env("HOME", home);
    cmd.args(["compiler", "install", "solar:0.3.0"])
        .assert_success()
        .stdout_eq(str![[r#"
solar 0.3.0: [..]/.foundry/solar/0.3.0/solar

"#]])
        .stderr_eq(str![[r#"
Installing solar 0.3.0

"#]]);

    cmd.forge_fuse().env("HOME", home);
    cmd.args(["compiler", "install", "solar:0.3.0", "--json"]).assert_success().stdout_eq(
        str![[
            r#"[{"compiler":"solar","version":"0.3.0","path":"[..]/.foundry/solar/0.3.0/solar"}]"#
        ]]
        .is_json(),
    );

    cmd.forge_fuse().env("HOME", home);
    cmd.args(["compiler", "list", "--json"])
        .assert_success()
        .stdout_eq(str![[r#"{"solc":"{...}","solar":["0.3.0"]}"#]].is_json());

    cmd.forge_fuse().env("HOME", home);
    cmd.args(["compiler", "remove", "solar:0.3.0"]).assert_success().stdout_eq("").stderr_eq(str![
        [r#"
Removed solar 0.3.0

"#]
    ]);

    cmd.forge_fuse().env("HOME", home);
    cmd.args(["compiler", "remove", "solar:0.3.0"]).assert_failure().stderr_eq(str![[r#"
Error: solar 0.3.0 is not installed

"#]]);
}

#[cfg(unix)]
#[forgetest]
fn compiler_remove_moves_svm_global_version(cmd: _) {
    let home = tempfile::tempdir().unwrap();
    let svm = home.path().join(".svm");
    for version in ["0.8.29", "0.8.30", "0.8.30-pre.1", "0.8.31-pre.1+commit.b59566f6"] {
        fs::create_dir_all(svm.join(version)).unwrap();
        fs::write(svm.join(version).join(format!("solc-{version}")), "").unwrap();
    }
    fs::write(svm.join(".global-version"), "0.8.30").unwrap();

    // A prerelease names another release, so the stable one and the global version stay.
    cmd.env("HOME", home.path());
    cmd.args(["compiler", "remove", "0.8.30-pre.1"]).assert_success().stderr_eq(str![[r#"
Removed solc 0.8.30-pre.1

"#]]);
    assert!(svm.join("0.8.30").exists());
    assert_eq!(fs::read_to_string(svm.join(".global-version")).unwrap(), "0.8.30");

    // A prerelease pinned with build metadata is installed under its full version.
    cmd.forge_fuse().env("HOME", home.path());
    cmd.args(["compiler", "remove", "0.8.31-pre.1+commit.b59566f6"]).assert_success().stderr_eq(
        str![[r#"
Removed solc 0.8.31-pre.1+commit.b59566f6

"#]],
    );

    // Build metadata names the same release.
    cmd.forge_fuse().env("HOME", home.path());
    cmd.args(["compiler", "remove", "0.8.30+commit.73712a01"]).assert_success().stderr_eq(str![[
        r#"
Removed solc 0.8.30

"#
    ]]);
    assert_eq!(fs::read_to_string(svm.join(".global-version")).unwrap(), "0.8.29");

    cmd.forge_fuse().env("HOME", home.path());
    cmd.args(["compiler", "remove", "0.8.29"]).assert_success();
    assert_eq!(fs::read_to_string(svm.join(".global-version")).unwrap(), "");
}
