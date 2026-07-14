//! Tests for the `forge compiler` command.

use foundry_compilers::compilers::solc::Solc;
use foundry_test_utils::{
    snapbox::IntoData,
    util::{SOLC_VERSION, get_vyper},
};

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

forgetest!(can_resolve_path, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);

    cmd.args(["compiler", "resolve", "--root", prj.root().to_str().unwrap()])
        .assert_success()
        .stdout_eq(str![[r#"
Solidity:
- 0.8.4


"#]]);
});

forgetest!(can_list_resolved_compiler_versions, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Solidity:
- 0.8.4


"#]]);
});

forgetest_init!(can_print_resolved_compiler_path, |prj, cmd| {
    prj.add_source("Contract", "contract Contract {}");
    let solc = Solc::find_svm_installed_version(&SOLC_VERSION.parse().unwrap()).unwrap().unwrap();

    cmd.args(["compiler", "resolve", "--path"])
        .assert_success()
        .stdout_eq(format!("{}\n", solc.solc.display()));
});

forgetest!(can_print_resolved_vyper_path, |prj, cmd| {
    let vyper = get_vyper();
    prj.add_raw_source("ICounter.vyi", VYPER_INTERFACE);
    prj.add_raw_source("Counter.vy", VYPER_CONTRACT);
    prj.update_config(|config| config.vyper.path = Some(vyper.path.clone()));

    cmd.args(["compiler", "resolve", "--path"])
        .assert_success()
        .stdout_eq(format!("{}\n", vyper.path.display()));
});

forgetest!(compiler_path_requires_single_version, |prj, cmd| {
    prj.add_source("ContractA", CONTRACT_A);
    prj.add_source("ContractB", CONTRACT_B);

    cmd.args(["compiler", "resolve", "--path"]).assert_failure().stdout_eq("").stderr_eq(
        "Error: multiple compilers resolved; use `forge compiler resolve` to inspect them\n",
    );
});

forgetest!(can_list_resolved_compiler_versions_json, |prj, cmd| {
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
});

forgetest!(can_list_resolved_compiler_versions_verbose, |prj, cmd| {
    prj.add_source("ContractC", CONTRACT_C);
    prj.add_source("ContractD", CONTRACT_D);

    cmd.args(["compiler", "resolve", "-v"]).assert_success().stdout_eq(str![[r#"
Solidity:

0.8.33:
├── src/ContractC.sol
└── src/ContractD.sol


"#]]);
});

forgetest!(can_list_resolved_compiler_versions_verbose_json, |prj, cmd| {
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
});

forgetest!(can_list_resolved_multiple_compiler_versions, |prj, cmd| {
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
});

forgetest!(can_list_resolved_multiple_compiler_versions_skipped, |prj, cmd| {
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
});

forgetest!(can_list_resolved_multiple_compiler_versions_skipped_json, |prj, cmd| {
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
});

forgetest!(can_list_resolved_multiple_compiler_versions_verbose, |prj, cmd| {
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
});

forgetest!(can_list_resolved_multiple_compiler_versions_verbose_json, |prj, cmd| {
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
});
