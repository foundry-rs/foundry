//! Tests for the `forge compiler` command.

use foundry_test_utils::snapbox::IntoData;

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
pragma solidity 0.8.30;

contract ContractC {}
"#;

const CONTRACT_D: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.30;

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

0.8.30:
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
      "version": "0.8.30",
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
- 0.8.30

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
      "version": "0.8.30",
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

0.8.30 (<= prague):
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
      "version": "0.8.30",
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
