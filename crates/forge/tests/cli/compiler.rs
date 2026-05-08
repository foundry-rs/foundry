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
pragma solidity 0.8.33;

contract ContractC {}
"#;

const CONTRACT_D: &str = r#"
// SPDX-license-identifier: MIT
pragma solidity 0.8.33;

contract ContractD {}
"#;

const VYPER_INTERFACE: &str = r#"
# pragma version >=0.5.0a1

@external
@view
def number() -> uint256:
    ...

@external
def set_number(new_number: uint256):
    ...

@external
def increment() -> uint256:
    ...
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
- 0.5.0


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

0.5.0:
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
      "version": "0.5.0",
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

0.5.0 (<= constantinople):
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
      "version": "0.5.0",
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

// Vyper `pragma version` regression coverage for foundry-rs/foundry-core#59. The released
// `foundry-compilers` silently dropped every Vyper version constraint; these tests pin the
// fixed parser end-to-end through the resolver against an installed Vyper 0.5.0a1 binary.

const VYPER_PEP440_COMPAT_RELEASE: &str = r#"
# pragma version ~=0.5.0a1

number: public(uint256)

@external
def setNumber(newNumber: uint256):
    self.number = newNumber
"#;

const VYPER_LEGACY_AT_VERSION: &str = r#"
#@version >=0.5.0a1

number: public(uint256)

@external
def setNumber(newNumber: uint256):
    self.number = newNumber
"#;

const VYPER_UNSATISFIABLE_PRAGMA: &str = r#"
# pragma version ==0.3.10

number: public(uint256)
"#;

// `~=0.5.0a1` (PEP 440 "compatible release") translates to `>=0.5.0-a1, <0.6.0`, satisfied by
// the installed Vyper 0.5.0a1.
forgetest!(vyper_pep440_compatible_release_pragma_resolves, |prj, cmd| {
    prj.add_raw_source("Counter.vy", VYPER_PEP440_COMPAT_RELEASE);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Vyper:
- 0.5.0


"#]]);
});

// Legacy `#@version <req>` spelling (only form supported prior to Vyper 0.3.10) must keep working.
forgetest!(vyper_legacy_at_version_pragma_resolves, |prj, cmd| {
    prj.add_raw_source("Counter.vy", VYPER_LEGACY_AT_VERSION);

    cmd.args(["compiler", "resolve"]).assert_success().stdout_eq(str![[r#"
Vyper:
- 0.5.0


"#]]);
});

// Sanity check that an unsatisfiable constraint now fails up front in the resolver instead of
// being silently dropped.
forgetest!(vyper_pragma_version_constraint_is_enforced, |prj, cmd| {
    prj.add_raw_source("Counter.vy", VYPER_UNSATISFIABLE_PRAGMA);

    cmd.args(["build"]).assert_failure().stderr_eq(str![[r#"
Error: Encountered invalid compiler version in src/Counter.vy: No compiler version exists that matches the version requirement: =0.3.10

"#]]);
});

// Beta and rc cases below: Vyper 0.5.0a1 cannot satisfy `=0.5.0-b1` or `=0.5.0-rc1`, so we
// assert the *translated* requirement string surfaces in the resolver error.

// `==0.5.0b1` → `=0.5.0-b1`
forgetest!(vyper_pep440_exact_beta_pragma_is_translated, |prj, cmd| {
    prj.add_raw_source(
        "Counter.vy",
        r#"
# pragma version ==0.5.0b1

number: public(uint256)
"#,
    );

    cmd.args(["build"]).assert_failure().stderr_eq(str![[r#"
Error: Encountered invalid compiler version in src/Counter.vy: No compiler version exists that matches the version requirement: =0.5.0-b1

"#]]);
});

// `==0.5.0rc1` → `=0.5.0-rc1`
forgetest!(vyper_pep440_exact_rc_pragma_is_translated, |prj, cmd| {
    prj.add_raw_source(
        "Counter.vy",
        r#"
# pragma version ==0.5.0rc1

number: public(uint256)
"#,
    );

    cmd.args(["build"]).assert_failure().stderr_eq(str![[r#"
Error: Encountered invalid compiler version in src/Counter.vy: No compiler version exists that matches the version requirement: =0.5.0-rc1

"#]]);
});
